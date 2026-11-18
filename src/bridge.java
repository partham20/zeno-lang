// The Java end of `import java` (docs/java.md).
//
// `zeno run` starts Java with this file as its program. It connects
// back to Zeno over a socket of this machine, then does what each line
// it is sent asks for, and answers with one line. The lines are the
// ones bridge.py reads and writes:
//
//     none, a bool, a number, a str            as JSON writes them
//     a float JSON cannot write                ["f", "nan"], "inf", "-inf"
//     a list                                   ["l", [item, ...]]
//     a dict                                   ["d", [[key, value], ...]]
//     anything else, which stays here          ["p", number]
//
// What stays here is kept in `kept` under its number until Zeno says
// that nobody there holds it any more.
//
// Java comes with nothing that reads JSON, so the reading and the
// writing of a line are here too. It is written for Java 11 and later.

import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.io.UncheckedIOException;
import java.lang.reflect.Array;
import java.lang.reflect.Constructor;
import java.lang.reflect.Executable;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.net.InetAddress;
import java.net.Socket;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.HashMap;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.TreeSet;

public class ZenoBridge {
    static final Map<Long, Object> kept = new HashMap<>();
    static long count;
    static Found found;
    /** The folder a path that does not begin at the root begins at: the
     * one the program is in. */
    static File home;

    /** Something this end has to say itself, as an error of the kind
     * Zeno knows it by. */
    static final class Told extends RuntimeException {
        final String kind;

        Told(String kind, String text) {
            super(text);
            this.kind = kind;
        }
    }

    /** What `java.type` gives: a class as its name is written in a
     * program of Java. A dot after it is something `static`, and calling
     * it is `new`. This is not the `Class` object, which
     * `thing.getClass()` gives and which has methods of its own. */
    static final class Type {
        final Class<?> of;

        Type(Class<?> of) {
            this.of = of;
        }
    }

    /** `thing.name` where `name` is a method: the method and what it
     * will be called on, kept until it is called. */
    static final class Bound {
        final Object on;
        final Class<?> in;
        final String name;

        Bound(Object on, Class<?> in, String name) {
            this.on = on;
            this.in = in;
            this.name = name;
        }
    }

    /** A list that Zeno sent, which is not yet a list or an array of
     * Java: which of the two depends on what is asked for. */
    static final class Listed {
        final List<Object> items = new ArrayList<>();
    }

    /** The same for a dict. */
    static final class Dicted {
        final List<Object[]> pairs = new ArrayList<>();
    }

    /** Where classes are found: what `CLASSPATH` names, or the folder
     * of the program, and then every jar `java.jar` adds. */
    static final class Found extends URLClassLoader {
        Found(ClassLoader above) {
            super(new URL[0], above);
        }

        void add(File place) throws IOException {
            addURL(place.toURI().toURL());
        }
    }

    // ------------------------------------------------------------ JSON

    static final class Reader {
        final String text;
        int at;

        Reader(String text) {
            this.text = text;
        }

        void blanks() {
            while (at < text.length() && " \t\r\n".indexOf(text.charAt(at)) >= 0) {
                at++;
            }
        }

        Object value() {
            blanks();
            char c = text.charAt(at);
            if (c == '{') {
                at++;
                Map<String, Object> members = new LinkedHashMap<>();
                blanks();
                if (text.charAt(at) == '}') {
                    at++;
                    return members;
                }
                while (true) {
                    blanks();
                    String name = string();
                    blanks();
                    at++; // the colon
                    members.put(name, value());
                    blanks();
                    if (text.charAt(at++) == '}') {
                        return members;
                    }
                }
            }
            if (c == '[') {
                at++;
                List<Object> items = new ArrayList<>();
                blanks();
                if (text.charAt(at) == ']') {
                    at++;
                    return items;
                }
                while (true) {
                    items.add(value());
                    blanks();
                    if (text.charAt(at++) == ']') {
                        return items;
                    }
                }
            }
            if (c == '"') {
                return string();
            }
            if (text.startsWith("true", at)) {
                at += 4;
                return Boolean.TRUE;
            }
            if (text.startsWith("false", at)) {
                at += 5;
                return Boolean.FALSE;
            }
            if (text.startsWith("null", at)) {
                at += 4;
                return null;
            }
            int from = at;
            boolean whole = true;
            while (at < text.length() && "+-0123456789.eE".indexOf(text.charAt(at)) >= 0) {
                whole &= ".eE".indexOf(text.charAt(at)) < 0;
                at++;
            }
            String number = text.substring(from, at);
            if (whole) {
                try {
                    return Long.valueOf(number);
                } catch (NumberFormatException tooLong) {
                    // Below: as a float.
                }
            }
            return Double.valueOf(number);
        }

        String string() {
            StringBuilder made = new StringBuilder();
            at++; // the opening quote
            while (true) {
                char c = text.charAt(at++);
                if (c == '"') {
                    return made.toString();
                }
                if (c != '\\') {
                    made.append(c);
                    continue;
                }
                char escaped = text.charAt(at++);
                switch (escaped) {
                    case 'n': made.append('\n'); break;
                    case 't': made.append('\t'); break;
                    case 'r': made.append('\r'); break;
                    case 'b': made.append('\b'); break;
                    case 'f': made.append('\f'); break;
                    case 'u':
                        made.append((char) Integer.parseInt(text.substring(at, at + 4), 16));
                        at += 4;
                        break;
                    default: made.append(escaped);
                }
            }
        }
    }

    static void write(Object value, StringBuilder line) {
        if (value == null) {
            line.append("null");
        } else if (value instanceof String) {
            String text = (String) value;
            line.append('"');
            for (int i = 0; i < text.length(); i++) {
                char c = text.charAt(i);
                if (c == '"' || c == '\\') {
                    line.append('\\').append(c);
                } else if (c == '\n') {
                    line.append("\\n");
                } else if (c < ' ' || c > '~') {
                    // Everything that is not plain goes by its number,
                    // half of a pair too.
                    line.append(String.format("\\u%04x", (int) c));
                } else {
                    line.append(c);
                }
            }
            line.append('"');
        } else if (value instanceof List) {
            line.append('[');
            String between = "";
            for (Object item : (List<?>) value) {
                line.append(between);
                write(item, line);
                between = ",";
            }
            line.append(']');
        } else if (value instanceof Map) {
            line.append('{');
            String between = "";
            for (Map.Entry<?, ?> member : ((Map<?, ?>) value).entrySet()) {
                line.append(between);
                write(member.getKey(), line);
                line.append(':');
                write(member.getValue(), line);
                between = ",";
            }
            line.append('}');
        } else {
            line.append(value);
        }
    }

    // ----------------------------------------------- Values, both ways

    static List<Object> tagged(String tag, Object inside) {
        List<Object> pair = new ArrayList<>();
        pair.add(tag);
        pair.add(inside);
        return pair;
    }

    static Object keep(Object thing) {
        count += 1;
        kept.put(count, thing);
        return tagged("p", count);
    }

    /** A value of Java, as it goes to Zeno. `null`, a boolean, a number
     * of any size up to `long` and `double`, a char and a String go as
     * themselves. Everything else stays here. */
    static Object out(Object value) {
        if (value == null || value instanceof Boolean || value instanceof String) {
            return value;
        }
        if (value instanceof Long || value instanceof Integer || value instanceof Short || value instanceof Byte) {
            return ((Number) value).longValue();
        }
        if (value instanceof Character) {
            return value.toString();
        }
        if (value instanceof Double || value instanceof Float) {
            // A float is written the short way it prints, 0.1 and not
            // 0.10000000149011612.
            double x = value instanceof Float ? Double.parseDouble(value.toString()) : (Double) value;
            if (Double.isNaN(x)) {
                return tagged("f", "nan");
            }
            if (Double.isInfinite(x)) {
                return tagged("f", x > 0 ? "inf" : "-inf");
            }
            return x;
        }
        return keep(value);
    }

    /** The same, but a list, a set, an array and a map go too, item by
     * item. */
    static Object copied(Object value, int depth) {
        if (depth > 200) {
            throw new Told("ValueError", "this is nested more than 200 deep, or holds itself, and cannot be copied");
        }
        if (value instanceof Map) {
            List<Object> pairs = new ArrayList<>();
            for (Map.Entry<?, ?> entry : ((Map<?, ?>) value).entrySet()) {
                Object key = out(entry.getKey());
                if (!(key instanceof String) && !(key instanceof Long)) {
                    throw new Told("ValueError", "a dict in Zeno has keys that are str or int, and this one has the key " + shown(entry.getKey()));
                }
                List<Object> pair = new ArrayList<>();
                pair.add(key);
                pair.add(copied(entry.getValue(), depth + 1));
                pairs.add(pair);
            }
            return tagged("d", pairs);
        }
        if (value instanceof Collection || (value != null && value.getClass().isArray())) {
            List<Object> items = new ArrayList<>();
            for (Object item : items(value)) {
                items.add(copied(item, depth + 1));
            }
            return tagged("l", items);
        }
        return out(value);
    }

    /** A value Zeno sent. A list and a dict are not made into anything
     * yet (`Listed`, `Dicted`), and an int is a `Long`. */
    static Object back(Object value) {
        if (!(value instanceof List)) {
            return value;
        }
        List<?> pair = (List<?>) value;
        String tag = (String) pair.get(0);
        Object inside = pair.get(1);
        if (tag.equals("p")) {
            if (!kept.containsKey(inside)) {
                throw new Told("Error", "this value was kept by a Java that has stopped since");
            }
            return kept.get(inside);
        }
        if (tag.equals("l")) {
            Listed listed = new Listed();
            for (Object item : (List<?>) inside) {
                listed.items.add(back(item));
            }
            return listed;
        }
        if (tag.equals("d")) {
            Dicted dicted = new Dicted();
            for (Object entry : (List<?>) inside) {
                List<?> both = (List<?>) entry;
                dicted.pairs.add(new Object[] {back(both.get(0)), back(both.get(1))});
            }
            return dicted;
        }
        if (inside.equals("nan")) {
            return Double.NaN;
        }
        if (inside.equals("inf")) {
            return Double.POSITIVE_INFINITY;
        }
        if (inside.equals("-inf")) {
            return Double.NEGATIVE_INFINITY;
        }
        return Double.valueOf((String) inside);
    }

    static boolean fitsInt(long n) {
        return n >= Integer.MIN_VALUE && n <= Integer.MAX_VALUE;
    }

    /** How well a value of Zeno fits a type that Java asks for: 0 is
     * what Java itself would pick, more is further, and -1 is not at
     * all. Java can have several methods of one name, and the one that
     * is called is the one the arguments fit best. */
    static int fit(Object value, Class<?> want) {
        if (value == null) {
            return want.isPrimitive() ? -1 : 0;
        }
        if (value instanceof Boolean) {
            return want == boolean.class ? 0 : want == Boolean.class ? 1 : want.isAssignableFrom(Boolean.class) ? 2 : -1;
        }
        if (value instanceof Long) {
            long n = (Long) value;
            if (want == int.class) return fitsInt(n) ? 0 : -1;
            if (want == long.class) return 1;
            if (want == short.class) return n >= Short.MIN_VALUE && n <= Short.MAX_VALUE ? 4 : -1;
            if (want == byte.class) return n >= Byte.MIN_VALUE && n <= Byte.MAX_VALUE ? 5 : -1;
            if (want == double.class) return 6;
            if (want == float.class) return 7;
            if (want == Integer.class) return fitsInt(n) ? 8 : -1;
            if (want == Long.class) return 9;
            if (want == Short.class) return n >= Short.MIN_VALUE && n <= Short.MAX_VALUE ? 10 : -1;
            if (want == Byte.class) return n >= Byte.MIN_VALUE && n <= Byte.MAX_VALUE ? 10 : -1;
            if (want == Double.class) return 11;
            if (want == Float.class) return 12;
            if (want == Object.class) return 14;
            return want.isAssignableFrom(fitsInt(n) ? Integer.class : Long.class) ? 13 : -1;
        }
        if (value instanceof Double) {
            if (want == double.class) return 0;
            if (want == Double.class) return 1;
            if (want == float.class) return 2;
            if (want == Float.class) return 3;
            if (want == Object.class) return 5;
            return want.isAssignableFrom(Double.class) ? 4 : -1;
        }
        if (value instanceof String) {
            if (want == String.class) return 0;
            if (want == Object.class) return 2;
            if (want.isAssignableFrom(String.class)) return 1;
            if (((String) value).length() == 1) {
                return want == char.class ? 3 : want == Character.class ? 4 : -1;
            }
            return -1;
        }
        if (value instanceof Listed) {
            if (want.isArray()) {
                int worst = 0;
                for (Object item : ((Listed) value).items) {
                    int one = fit(item, want.getComponentType());
                    if (one < 0) return -1;
                    worst = Math.max(worst, one);
                }
                return 2 + worst;
            }
            return want == Object.class ? 3 : want.isAssignableFrom(ArrayList.class) ? 1 : -1;
        }
        if (value instanceof Dicted) {
            return want == Object.class ? 3 : want.isAssignableFrom(LinkedHashMap.class) ? 1 : -1;
        }
        if (value instanceof Type) {
            return want == Class.class ? 0 : want == Object.class ? 1 : -1;
        }
        if (value instanceof Bound) {
            return -1;
        }
        if (want.isPrimitive()) {
            return -1;
        }
        return want == value.getClass() ? 0 : want == Object.class ? 2 : want.isInstance(value) ? 1 : -1;
    }

    /** The value, as the type that was asked for. `fit` has said that
     * it can be. Where nothing more is asked than an Object, an int is
     * an Integer if it fits one and a Long if not, as a number written
     * in Java is. */
    static Object turn(Object value, Class<?> want) {
        if (value instanceof Long) {
            long n = (Long) value;
            if (want == int.class || want == Integer.class) return (int) n;
            if (want == long.class || want == Long.class) return n;
            if (want == short.class || want == Short.class) return (short) n;
            if (want == byte.class || want == Byte.class) return (byte) n;
            if (want == double.class || want == Double.class) return (double) n;
            if (want == float.class || want == Float.class) return (float) n;
            return fitsInt(n) ? (Object) (int) n : (Object) n;
        }
        if (value instanceof Double) {
            return want == float.class || want == Float.class ? (Object) ((Double) value).floatValue() : value;
        }
        if (value instanceof String) {
            return want == char.class || want == Character.class ? (Object) ((String) value).charAt(0) : value;
        }
        if (value instanceof Listed) {
            List<Object> items = ((Listed) value).items;
            if (want.isArray()) {
                Object array = Array.newInstance(want.getComponentType(), items.size());
                for (int i = 0; i < items.size(); i++) {
                    Array.set(array, i, turn(items.get(i), want.getComponentType()));
                }
                return array;
            }
            List<Object> list = new ArrayList<>();
            for (Object item : items) {
                list.add(turn(item, Object.class));
            }
            return list;
        }
        if (value instanceof Dicted) {
            Map<Object, Object> map = new LinkedHashMap<>();
            for (Object[] pair : ((Dicted) value).pairs) {
                map.put(turn(pair[0], Object.class), turn(pair[1], Object.class));
            }
            return map;
        }
        if (value instanceof Type) {
            return ((Type) value).of;
        }
        return value;
    }

    // ------------------------------------------- Methods and classes

    /** The arguments as one method or constructor takes them, or null
     * if it does not take them. `score[0]` is how well they fit. */
    static Object[] taken(Executable by, List<Object> args, int[] score) {
        Class<?>[] wants = by.getParameterTypes();
        if (wants.length == args.size()) {
            int total = 0;
            for (int i = 0; i < wants.length && total >= 0; i++) {
                int one = fit(args.get(i), wants[i]);
                total = one < 0 ? -1 : total + one;
            }
            if (total >= 0) {
                Object[] turned = new Object[wants.length];
                for (int i = 0; i < wants.length; i++) {
                    turned[i] = turn(args.get(i), wants[i]);
                }
                score[0] = total;
                return turned;
            }
        }
        // `String.format(pattern, Object...)`: what comes after the
        // ones it names is gathered into the last, an array.
        if (by.isVarArgs() && args.size() >= wants.length - 1) {
            int fixed = wants.length - 1;
            Class<?> each = wants[fixed].getComponentType();
            int total = 100;
            for (int i = 0; i < args.size() && total >= 0; i++) {
                int one = fit(args.get(i), i < fixed ? wants[i] : each);
                total = one < 0 ? -1 : total + one;
            }
            if (total >= 0) {
                Object[] turned = new Object[wants.length];
                for (int i = 0; i < fixed; i++) {
                    turned[i] = turn(args.get(i), wants[i]);
                }
                Object rest = Array.newInstance(each, args.size() - fixed);
                for (int i = fixed; i < args.size(); i++) {
                    Array.set(rest, i - fixed, turn(args.get(i), each));
                }
                turned[fixed] = rest;
                score[0] = total;
                return turned;
            }
        }
        return null;
    }

    static boolean open(Class<?> c) {
        return Modifier.isPublic(c.getModifiers()) && c.getModule().isExported(c.getPackageName());
    }

    /** The method as a class that may be used from outside declares it.
     * What `List.of(1, 2)` gives is of a class that is nobody's
     * business, and `size` cannot be called through that class, but it
     * can through `List`, which the class is one of. */
    static Method reachable(Method method) {
        if (open(method.getDeclaringClass())) {
            return method;
        }
        ArrayDeque<Class<?>> above = new ArrayDeque<>();
        above.add(method.getDeclaringClass());
        while (!above.isEmpty()) {
            Class<?> c = above.remove();
            if (open(c)) {
                try {
                    return c.getMethod(method.getName(), method.getParameterTypes());
                } catch (NoSuchMethodException notHere) {
                    // Further up.
                }
            }
            if (c.getSuperclass() != null) {
                above.add(c.getSuperclass());
            }
            above.addAll(Arrays.asList(c.getInterfaces()));
        }
        try {
            // A class of the program's own jar that is not public.
            method.setAccessible(true);
            return method;
        } catch (RuntimeException closed) {
            return null;
        }
    }

    static String written(Executable one, String name) {
        List<String> types = new ArrayList<>();
        for (Class<?> want : one.getParameterTypes()) {
            types.add(want.getSimpleName());
        }
        if (one.isVarArgs()) {
            String last = types.get(types.size() - 1);
            types.set(types.size() - 1, last.substring(0, last.length() - 2) + "...");
        }
        return name + "(" + String.join(", ", types) + ")";
    }

    static String kind(Object value) {
        if (value == null) return "none";
        if (value instanceof Boolean) return "a bool";
        if (value instanceof Long) return "an int";
        if (value instanceof Double) return "a float";
        if (value instanceof String) return "a str";
        if (value instanceof Listed) return "a list";
        if (value instanceof Dicted) return "a dict";
        if (value instanceof Type) return "the class " + ((Type) value).of.getSimpleName();
        if (value instanceof Bound) return "a method";
        return "a " + value.getClass().getSimpleName();
    }

    /** The one of `all` that the arguments fit best, called. */
    static Object called(List<? extends Executable> all, Object on, List<Object> args, String name) throws Throwable {
        Executable best = null;
        Object[] with = null;
        int least = Integer.MAX_VALUE;
        boolean two = false;
        TreeSet<String> every = new TreeSet<>();
        for (Executable one : all) {
            every.add(written(one, name));
            int[] score = new int[1];
            Object[] turned = taken(one, args, score);
            if (turned == null) {
                continue;
            }
            if (score[0] < least) {
                best = one;
                with = turned;
                least = score[0];
                two = false;
            } else if (score[0] == least && !Arrays.equals(one.getParameterTypes(), best.getParameterTypes())) {
                two = true;
            }
        }
        if (best == null || two) {
            List<String> given = new ArrayList<>();
            for (Object arg : args) {
                given.add(kind(arg));
            }
            String what = given.isEmpty() ? "no arguments" : String.join(", ", given);
            throw new Told(
                "Error",
                (best == null ? "Java has no `" + name + "` that takes " + what : "Java has more than one `" + name + "` that takes " + what + ", and nothing says which is meant")
                    + ". There is " + String.join(", ", every));
        }
        try {
            if (best instanceof Constructor) {
                return ((Constructor<?>) best).newInstance(with);
            }
            return ((Method) best).invoke(on, with);
        } catch (InvocationTargetException inside) {
            throw inside.getCause();
        }
    }

    static List<Method> methods(Class<?> in, String name, boolean onlyStatic) {
        List<Method> all = new ArrayList<>();
        LinkedHashSet<String> seen = new LinkedHashSet<>();
        for (Method method : in.getMethods()) {
            if (!method.getName().equals(name) || (onlyStatic && !Modifier.isStatic(method.getModifiers()))) {
                continue;
            }
            // The same method can come from two interfaces.
            if (!seen.add(Arrays.toString(method.getParameterTypes()))) {
                continue;
            }
            Method usable = reachable(method);
            if (usable != null) {
                all.add(usable);
            }
        }
        return all;
    }

    static final Map<String, Class<?>> PLAIN = new HashMap<>();

    static {
        for (Class<?> plain : new Class<?>[] {int.class, long.class, double.class, float.class, boolean.class, char.class, byte.class, short.class}) {
            PLAIN.put(plain.getName(), plain);
        }
    }

    /** A class by its whole name, `java.util.ArrayList`. A class inside
     * a class is found as Java writes it, `java.util.Map.Entry`. */
    static Class<?> named(String name) {
        if (PLAIN.containsKey(name)) {
            return PLAIN.get(name);
        }
        String tried = name;
        while (true) {
            try {
                return Class.forName(tried, true, found);
            } catch (ClassNotFoundException | NoClassDefFoundError notThere) {
                int dot = tried.lastIndexOf('.');
                if (dot < 0) {
                    return null;
                }
                tried = tried.substring(0, dot) + "$" + tried.substring(dot + 1);
            }
        }
    }

    static File placed(String path) {
        File file = new File(path);
        return file.isAbsolute() ? file : new File(home, path);
    }

    /** A jar, or a folder: its classes, and every jar in it. */
    static void add(File place) throws IOException {
        if (!place.exists()) {
            throw new Told("FileError", "there is no jar or folder `" + place.getPath() + "`");
        }
        found.add(place);
        File[] inside = place.isDirectory() ? place.listFiles() : null;
        if (inside != null) {
            Arrays.sort(inside);
            for (File jar : inside) {
                if (jar.isFile() && jar.getName().endsWith(".jar")) {
                    found.add(jar);
                }
            }
        }
    }

    // ------------------------------------------------ What Zeno asks

    static String described(Object thing) {
        if (thing instanceof Type) return "the class " + ((Type) thing).of.getName();
        if (thing instanceof Bound) return "a method";
        return (thing.getClass().isArray() ? "an array " : "a ") + thing.getClass().getTypeName();
    }

    static Object there(Object thing, String doing) {
        if (thing == null) {
            throw new Told("Error", "this is `null` in Java, and " + doing);
        }
        return thing;
    }

    static Object get(Object thing, String name) throws Throwable {
        boolean isType = thing instanceof Type;
        Class<?> in = isType ? ((Type) thing).of : thing.getClass();
        if (!isType && in.isArray() && name.equals("length")) {
            return (long) Array.getLength(thing);
        }
        if (!(thing instanceof Bound)) {
            try {
                Field field = in.getField(name);
                if (!isType || Modifier.isStatic(field.getModifiers())) {
                    return out(field.get(isType ? null : thing));
                }
            } catch (NoSuchFieldException | IllegalAccessException notOne) {
                // A method, then.
            }
            if (!methods(in, name, isType).isEmpty()) {
                return keep(new Bound(isType ? null : thing, in, name));
            }
            if (isType) {
                Class<?> inner = named(in.getName() + "." + name);
                if (inner != null) {
                    return keep(new Type(inner));
                }
            }
        }
        throw new Told("Error", described(thing) + " has no " + (isType ? "static " : "") + "field or method `" + name + "`");
    }

    static void set(Object thing, String name, Object to) throws Throwable {
        boolean isType = thing instanceof Type;
        Class<?> in = isType ? ((Type) thing).of : thing.getClass();
        Field field;
        try {
            field = in.getField(name);
        } catch (NoSuchFieldException notOne) {
            throw new Told("Error", described(thing) + " has no field `" + name + "` that can be set");
        }
        if (fit(to, field.getType()) < 0) {
            throw new Told("Error", "the field `" + name + "` is a " + field.getType().getSimpleName() + ", and cannot be set to " + kind(to));
        }
        field.set(isType ? null : thing, turn(to, field.getType()));
    }

    static Object call(Object thing, List<Object> args) throws Throwable {
        if (thing instanceof Type) {
            Class<?> in = ((Type) thing).of;
            String why = in.isInterface() ? "is an interface"
                : in.isPrimitive() ? "is not a class"
                : Modifier.isAbstract(in.getModifiers()) ? "is abstract"
                : in.getConstructors().length == 0 ? "has no constructor that others may use" : null;
            if (why != null) {
                throw new Told("Error", "`" + in.getSimpleName() + "` " + why + ", and no object can be made of it by calling it");
            }
            return called(Arrays.asList(in.getConstructors()), null, args, "new " + in.getSimpleName());
        }
        if (thing instanceof Bound) {
            Bound bound = (Bound) thing;
            return called(methods(bound.in, bound.name, bound.on == null), bound.on, args, bound.name);
        }
        throw new Told("Error", described(thing) + " is not a method or a class, and cannot be called");
    }

    static int index(Object at, int length) {
        if (!(at instanceof Long)) {
            throw new Told("Error", "the place of an item is an int, but this is " + kind(at));
        }
        long n = (Long) at;
        if (n < 0 || n >= length) {
            throw new Told("IndexError", "index " + n + " is out of range for " + length + " items");
        }
        return (int) n;
    }

    static Object item(Object thing, Object at) {
        if (thing.getClass().isArray()) {
            return Array.get(thing, index(at, Array.getLength(thing)));
        }
        if (thing instanceof List) {
            List<?> list = (List<?>) thing;
            return list.get(index(at, list.size()));
        }
        if (thing instanceof Map) {
            return ((Map<?, ?>) thing).get(turn(at, Object.class));
        }
        if (thing instanceof CharSequence) {
            CharSequence text = (CharSequence) thing;
            return text.charAt(index(at, text.length()));
        }
        throw new Told("Error", described(thing) + " has no items to take by `[ ]`");
    }

    @SuppressWarnings("unchecked")
    static void setItem(Object thing, Object at, Object to) {
        if (thing.getClass().isArray()) {
            Class<?> each = thing.getClass().getComponentType();
            if (fit(to, each) < 0) {
                throw new Told("Error", "this is an array of " + each.getSimpleName() + ", and " + kind(to) + " cannot be put in it");
            }
            Array.set(thing, index(at, Array.getLength(thing)), turn(to, each));
        } else if (thing instanceof List) {
            List<Object> list = (List<Object>) thing;
            list.set(index(at, list.size()), turn(to, Object.class));
        } else if (thing instanceof Map) {
            ((Map<Object, Object>) thing).put(turn(at, Object.class), turn(to, Object.class));
        } else {
            throw new Told("Error", described(thing) + " has no items to set by `[ ]`");
        }
    }

    static long length(Object thing) {
        if (thing.getClass().isArray()) return Array.getLength(thing);
        if (thing instanceof Collection) return ((Collection<?>) thing).size();
        if (thing instanceof Map) return ((Map<?, ?>) thing).size();
        if (thing instanceof CharSequence) return ((CharSequence) thing).length();
        throw new Told("Error", described(thing) + " has no length");
    }

    static boolean has(Object thing, Object item) {
        Object wanted = turn(item, Object.class);
        if (thing instanceof Collection) return ((Collection<?>) thing).contains(wanted);
        if (thing instanceof Map) return ((Map<?, ?>) thing).containsKey(wanted);
        if (thing instanceof CharSequence && wanted instanceof String) return thing.toString().contains((String) wanted);
        if (thing.getClass().isArray()) {
            for (Object one : items(thing)) {
                if (Objects.equals(one, wanted)) {
                    return true;
                }
            }
            return false;
        }
        throw new Told("Error", described(thing) + " cannot be asked what is `in` it");
    }

    /** Whatever Java can go through, taken now, all of it. A map is
     * gone through by its keys, as a dict is. */
    static List<Object> items(Object thing) {
        List<Object> all = new ArrayList<>();
        if (thing.getClass().isArray()) {
            for (int i = 0; i < Array.getLength(thing); i++) {
                all.add(Array.get(thing, i));
            }
            return all;
        }
        Iterator<?> each;
        if (thing instanceof Map) {
            each = ((Map<?, ?>) thing).keySet().iterator();
        } else if (thing instanceof Iterable) {
            each = ((Iterable<?>) thing).iterator();
        } else if (thing instanceof Iterator) {
            each = (Iterator<?>) thing;
        } else if (thing instanceof java.util.stream.BaseStream) {
            each = ((java.util.stream.BaseStream<?, ?>) thing).iterator();
        } else if (thing instanceof java.util.Enumeration) {
            each = ((java.util.Enumeration<?>) thing).asIterator();
        } else {
            throw new Told("Error", described(thing) + " cannot be gone through with `for`");
        }
        while (each.hasNext()) {
            all.add(each.next());
        }
        return all;
    }

    /** How `print` shows it: as Java's own `toString` writes it. An
     * array has none worth reading, so its items are shown. */
    static String shown(Object thing) {
        if (thing instanceof Type) return "class " + ((Type) thing).of.getName();
        if (thing instanceof Bound) return "method " + ((Bound) thing).in.getName() + "." + ((Bound) thing).name;
        if (thing != null && thing.getClass().isArray()) {
            String all = Arrays.deepToString(new Object[] {thing});
            return all.substring(1, all.length() - 1);
        }
        return String.valueOf(thing);
    }

    static boolean equal(Object thing, Object other) {
        if (thing instanceof Type && other instanceof Type) {
            return ((Type) thing).of == ((Type) other).of;
        }
        return Objects.equals(thing, turn(other, Object.class));
    }

    @SuppressWarnings("unchecked")
    static Object act(Map<String, Object> ask) throws Throwable {
        String op = (String) ask.get("op");
        String name = (String) ask.get("name");
        if (op.equals("import")) {
            Class<?> c = named(name);
            if (c == null) {
                throw new Told(
                    "FileError",
                    "Java has no class `" + name + "`. A class is asked for by its whole name, as `java.util.ArrayList`. "
                        + "One that does not come with Java is in a jar, which `java.jar` adds. This Java is " + System.getProperty("java.home"));
            }
            return keep(new Type(c));
        }
        if (op.equals("jar")) {
            add(placed(name));
            return null;
        }
        Object thing = back(ask.get("on"));
        if (op.equals("equal")) {
            return equal(thing, back(ask.get("to")));
        }
        if (op.equals("str") || op.equals("repr")) {
            return shown(thing);
        }
        if (op.equals("get")) {
            return get(there(thing, "has no `" + name + "`"), name);
        }
        if (op.equals("set")) {
            set(there(thing, "has no `" + name + "`"), name, back(ask.get("to")));
            return null;
        }
        if (op.equals("call")) {
            List<Object> named = (List<Object>) ask.get("named");
            if (!named.isEmpty()) {
                throw new Told("Error", "Java has no named arguments, and `" + ((List<Object>) named.get(0)).get(0) + "` was given as one");
            }
            List<Object> args = new ArrayList<>();
            for (Object arg : (List<Object>) ask.get("args")) {
                args.add(back(arg));
            }
            return out(call(there(thing, "cannot be called"), args));
        }
        there(thing, "has nothing in it");
        if (op.equals("item")) {
            Object at = back(ask.get("at"));
            // `System["in"]`: on what has no items of its own, a name
            // in brackets is the name after a dot. It is the way to a
            // name that is a word of Zeno.
            boolean holds = thing.getClass().isArray() || thing instanceof List || thing instanceof Map || thing instanceof CharSequence;
            if (at instanceof String && !holds) {
                return get(thing, (String) at);
            }
            return out(item(thing, at));
        }
        if (op.equals("set_item")) {
            setItem(thing, back(ask.get("at")), back(ask.get("to")));
            return null;
        }
        if (op.equals("len")) {
            return length(thing);
        }
        if (op.equals("has")) {
            return has(thing, back(ask.get("item")));
        }
        if (op.equals("items")) {
            List<Object> all = new ArrayList<>();
            for (Object item : items(thing)) {
                all.add(out(item));
            }
            return tagged("l", all);
        }
        if (op.equals("copy")) {
            return copied(thing, 0);
        }
        throw new Told("Error", "Zeno asked for `" + op + "`, which this Java end does not know");
    }

    /** The kind of error Zeno raises for one of Java: an `IndexError`
     * for a place that is not there, a `ValueError` for an argument
     * that cannot be used, a `FileError` for what a file or the network
     * refused, and `Error` for the rest. */
    static String kindOf(Throwable error) {
        if (error instanceof Told) return ((Told) error).kind;
        if (error instanceof IndexOutOfBoundsException) return "IndexError";
        if (error instanceof IllegalArgumentException) return "ValueError";
        if (error instanceof IOException || error instanceof UncheckedIOException) return "FileError";
        return "Error";
    }

    static String said(Throwable error) {
        if (error instanceof Told) {
            return error.getMessage();
        }
        String name = error.getClass().getSimpleName();
        return error.getMessage() == null ? name : name + ": " + error.getMessage();
    }

    public static void main(String[] words) throws Exception {
        String[] told = System.getenv("ZENO_BRIDGE").split(" ");
        count = Long.parseLong(told[2]);
        String program = System.getenv("ZENO_PROGRAM");
        home = new File(program == null || program.isEmpty() ? System.getProperty("user.dir") : program);

        // Where Java itself would look: what `CLASSPATH` names, and
        // with none, the folder of the program.
        found = new Found(ZenoBridge.class.getClassLoader());
        String paths = System.getenv("CLASSPATH");
        if (paths == null || paths.isEmpty()) {
            found.add(home);
        } else {
            for (String path : paths.split(File.pathSeparator)) {
                if (path.endsWith("*")) {
                    File folder = placed(path.substring(0, path.length() - 1));
                    if (folder.isDirectory()) {
                        add(folder);
                    }
                } else if (!path.isEmpty() && placed(path).exists()) {
                    found.add(placed(path));
                }
            }
        }
        Thread.currentThread().setContextClassLoader(found);

        Socket link = new Socket(InetAddress.getByName("127.0.0.1"), Integer.parseInt(told[0]));
        link.setTcpNoDelay(true);
        BufferedReader lines = new BufferedReader(new InputStreamReader(link.getInputStream(), StandardCharsets.UTF_8));
        OutputStream answers = link.getOutputStream();

        Map<String, Object> hello = new LinkedHashMap<>();
        hello.put("hello", told[1]);
        hello.put("version", System.getProperty("java.version"));
        hello.put("program", System.getProperty("java.home"));
        StringBuilder first = new StringBuilder();
        write(hello, first);
        answers.write(first.append('\n').toString().getBytes(StandardCharsets.UTF_8));
        answers.flush();

        String line;
        while (true) {
            try {
                line = lines.readLine();
            } catch (IOException gone) {
                break;
            }
            if (line == null) {
                break;
            }
            Map<String, Object> answer = new LinkedHashMap<>();
            try {
                @SuppressWarnings("unchecked")
                Map<String, Object> ask = (Map<String, Object>) new Reader(line).value();
                Object free = ask.get("free");
                if (free != null) {
                    for (Object number : (List<?>) free) {
                        kept.remove(number);
                    }
                }
                answer.put("ok", act(ask));
            } catch (Throwable error) {
                if (error instanceof ExceptionInInitializerError && error.getCause() != null) {
                    error = error.getCause();
                }
                List<Object> err = new ArrayList<>();
                err.add(kindOf(error));
                err.add(said(error));
                answer.clear();
                answer.put("err", err);
            }
            // What Java printed is on the screen before Zeno goes on.
            System.out.flush();
            System.err.flush();
            StringBuilder reply = new StringBuilder();
            write(answer, reply);
            try {
                answers.write(reply.append('\n').toString().getBytes(StandardCharsets.UTF_8));
                answers.flush();
            } catch (IOException gone) {
                break;
            }
        }
        // Zeno is done, or gone: there is nobody to answer.
        System.exit(0);
    }
}
