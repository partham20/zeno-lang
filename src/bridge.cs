// The .NET end of `import csharp` (docs/csharp.md).
//
// `zeno run` builds this once into a program of .NET, and starts it.
// It connects back to Zeno over a socket of this machine, then does
// what each line it is sent asks for, and answers with one line. The
// lines are the ones bridge.py reads and writes:
//
//     none, a bool, an int, a float, a str     as JSON writes them
//     a float JSON cannot write                ["f", "nan"], "inf", "-inf"
//     a list                                   ["l", [item, ...]]
//     a dict                                   ["d", [[key, value], ...]]
//     anything else, which stays here          ["p", number]
//
// What stays here is kept in Kept under its number until Zeno says
// that nobody there holds it any more.

using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Net.Sockets;
using System.Reflection;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;

namespace Zeno
{
    // Something this end has to say itself, as an error of the kind Zeno
    // knows it by.
    sealed class Told : Exception
    {
        public readonly string Kind;
        public Told(string kind, string text) : base(text) { Kind = kind; }
    }

    // `csharp.type("System.Text.StringBuilder")`: a type by its whole
    // name. .NET has `Type` for that, and a `Type` is also a value of its
    // own (`thing.GetType()`), so the one Zeno asked for by name is kept
    // in this.
    sealed class Named
    {
        public readonly Type Type;
        public Named(Type type) { Type = type; }
    }

    // `thing.Name`: the methods of that name and what they will be called
    // on (null for a static one), kept until one is called.
    sealed class Bound
    {
        public readonly object On;
        public readonly Type Type;
        public readonly string Name;
        public Bound(object on, Type type, string name) { On = on; Type = type; Name = name; }
    }

    static class Bridge
    {
        static readonly Dictionary<long, object> Kept = new Dictionary<long, object>();
        static long Count;
        static string Home = "";
        static readonly List<Assembly> Loaded = new List<Assembly>();
        static readonly List<string> Folders = new List<string>();
        // The .dll files of the packages of NuGet the program has, by
        // the name of each (Packages).
        static readonly Dictionary<string, string> Package = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);

        static object[] Keep(object thing)
        {
            Count += 1;
            Kept[Count] = thing;
            return new object[] { "p", Count };
        }

        static bool IsWhole(object value) =>
            value is sbyte || value is byte || value is short || value is ushort || value is int || value is uint || value is long;

        // A value of .NET, as it goes to Zeno. null, a bool, a whole
        // number that fits 64 bits, a float, a char and a string go as
        // themselves. Everything else stays here, an array too.
        static object Out(object value)
        {
            if (value == null || value is bool || value is string) return value is string s && !IsText(s) ? Keep(value) : value;
            if (IsWhole(value)) return Convert.ToInt64(value, CultureInfo.InvariantCulture);
            if (value is ulong big) return big <= long.MaxValue ? (object)(long)big : Keep(value);
            if (value is char c) return char.IsSurrogate(c) ? Keep(value) : c.ToString();
            if (value is float || value is double)
            {
                double x = Convert.ToDouble(value, CultureInfo.InvariantCulture);
                if (double.IsNaN(x)) return new object[] { "f", "nan" };
                if (double.IsInfinity(x)) return new object[] { "f", x > 0 ? "inf" : "-inf" };
                return x;
            }
            return Keep(value);
        }

        // A string with half of a pair of UTF-16 in it cannot be UTF-8.
        static bool IsText(string s)
        {
            for (int i = 0; i < s.Length; i++)
            {
                if (char.IsHighSurrogate(s[i]) && i + 1 < s.Length && char.IsLowSurrogate(s[i + 1])) { i++; continue; }
                if (char.IsSurrogate(s[i])) return false;
            }
            return true;
        }

        // `csharp.copy`: what can be gone through, as a list, and a
        // dictionary as a dict, with everything in it.
        static object Copied(object value, int depth = 0)
        {
            if (depth > 200) throw new Told("ValueError", "this is nested more than 200 deep, or holds itself, and cannot be copied");
            if (value is IDictionary dictionary)
            {
                var pairs = new List<object>();
                foreach (DictionaryEntry entry in dictionary)
                {
                    object key = Out(entry.Key);
                    if (!(key is string || key is long)) throw new Told("ValueError", "a dict in Zeno has keys that are str or int, and this one has the key " + Shown(entry.Key));
                    pairs.Add(new object[] { key, Copied(entry.Value, depth + 1) });
                }
                return new object[] { "d", pairs };
            }
            if (value is IEnumerable items && !(value is string))
            {
                var made = new List<object>();
                foreach (object item in items)
                {
                    if (item != null && item.GetType().IsGenericType && item.GetType().GetGenericTypeDefinition() == typeof(KeyValuePair<,>))
                        throw new Told("ValueError", "a pair cannot be copied by itself. `csharp.copy` of the dictionary it is in makes a dict");
                    made.Add(Copied(item, depth + 1));
                }
                return new object[] { "l", made };
            }
            return Out(value);
        }

        // A value Zeno sent, as a value of .NET: a list is a List<object>
        // and a dict a Dictionary<object, object>, which a method that
        // wants something else is given turned into it (Turn).
        static object Back(JsonElement value)
        {
            switch (value.ValueKind)
            {
                case JsonValueKind.Null: return null;
                case JsonValueKind.True: return true;
                case JsonValueKind.False: return false;
                case JsonValueKind.String: return value.GetString();
                case JsonValueKind.Number:
                    return value.TryGetInt64(out long whole) && !value.GetRawText().Contains('.') && !value.GetRawText().Contains('e') && !value.GetRawText().Contains('E')
                        ? (object)whole : value.GetDouble();
            }
            string tag = value[0].GetString();
            JsonElement inside = value[1];
            switch (tag)
            {
                case "p":
                    long id = inside.GetInt64();
                    if (!Kept.TryGetValue(id, out object thing)) throw new Told("Error", "this value was kept by a .NET that has stopped since");
                    return thing;
                case "l":
                    return inside.EnumerateArray().Select(Back).ToList();
                case "d":
                    var made = new Dictionary<object, object>();
                    foreach (JsonElement pair in inside.EnumerateArray()) made[Back(pair[0])] = Back(pair[1]);
                    return made;
                default:
                    string word = inside.GetString();
                    return word == "nan" ? double.NaN : word == "inf" ? double.PositiveInfinity : double.NegativeInfinity;
            }
        }

        // ------------------------------------------------- Choosing a method

        // How well a value fits a parameter of this type: 0 is exactly,
        // more is less well, and -1 is not at all.
        static int Fit(object value, Type want)
        {
            if (want.IsByRef) want = want.GetElementType();
            if (value == null) return !want.IsValueType || Nullable.GetUnderlyingType(want) != null ? 1 : -1;
            // `int?` takes what `int` takes, a little less well.
            if (Nullable.GetUnderlyingType(want) is Type inner)
            {
                int fit = Fit(value, inner);
                return fit < 0 ? -1 : fit + 1;
            }
            // A type Zeno asked for by name is a `Type` where one is wanted.
            if (value is Named && want.IsAssignableFrom(typeof(Type))) return want == typeof(Type) ? 0 : 1;
            Type plain = Nullable.GetUnderlyingType(want) ?? want;
            Type have = value.GetType();
            if (plain == have) return 0;
            if (value is long n)
            {
                if (plain == typeof(int)) return n >= int.MinValue && n <= int.MaxValue ? 1 : -1;
                if (plain == typeof(short)) return n >= short.MinValue && n <= short.MaxValue ? 3 : -1;
                if (plain == typeof(byte)) return n >= byte.MinValue && n <= byte.MaxValue ? 3 : -1;
                if (plain == typeof(sbyte)) return n >= sbyte.MinValue && n <= sbyte.MaxValue ? 3 : -1;
                if (plain == typeof(ushort)) return n >= ushort.MinValue && n <= ushort.MaxValue ? 3 : -1;
                if (plain == typeof(uint)) return n >= 0 && n <= uint.MaxValue ? 2 : -1;
                if (plain == typeof(ulong)) return n >= 0 ? 2 : -1;
                if (plain == typeof(double)) return 4;
                if (plain == typeof(float)) return 5;
                if (plain == typeof(decimal)) return 4;
                if (plain.IsEnum) return 6;
            }
            if (value is double)
            {
                if (plain == typeof(float)) return 1;
                if (plain == typeof(decimal)) return 2;
            }
            if (value is string s)
            {
                if (plain == typeof(char)) return s.Length == 1 ? 2 : -1;
            }
            if (plain.IsInstanceOfType(value)) return plain == typeof(object) ? 8 : 1;
            // A list goes to an array or a collection of .NET, as well as
            // its items go to theirs, and to one that is a class sooner
            // than to an interface.
            if (value is IList list && (plain.IsArray || IsListLike(plain)))
            {
                int worst = Worst(list.Cast<object>(), plain.IsArray ? plain.GetElementType() : plain.GetGenericArguments()[0]);
                return worst < 0 ? -1 : worst + (plain.IsInterface ? 4 : 3);
            }
            if (value is IDictionary dictionary && IsDictionaryLike(plain))
            {
                Type[] both = plain.GetGenericArguments();
                int keys = Worst(dictionary.Keys.Cast<object>(), both[0]);
                int values = Worst(dictionary.Values.Cast<object>(), both[1]);
                return keys < 0 || values < 0 ? -1 : Math.Max(keys, values) + (plain.IsInterface ? 4 : 3);
            }
            return -1;
        }

        // `Task.FromResult<T>(T value)`: what each type parameter is,
        // from the arguments, as C# works it out. One that no argument
        // says is `object`.
        static MethodInfo Inferred(MethodInfo method, object[] args)
        {
            Type[] generic = method.GetGenericArguments();
            var bound = new Type[generic.Length];
            ParameterInfo[] parameters = method.GetParameters();
            for (int i = 0; i < parameters.Length && i < args.Length; i++) Bind(parameters[i].ParameterType, args[i], generic, bound);
            for (int i = 0; i < bound.Length; i++) bound[i] = bound[i] ?? typeof(object);
            try { return method.MakeGenericMethod(bound); }
            catch (ArgumentException) { return null; }
        }

        static void Bind(Type parameter, object value, Type[] generic, Type[] bound)
        {
            if (value == null) return;
            if (parameter.IsGenericParameter)
            {
                int at = Array.IndexOf(generic, parameter);
                if (at >= 0 && bound[at] == null) bound[at] = value.GetType();
                return;
            }
            if (value is IList list && list.Count > 0 && (parameter.IsArray || IsListLike(parameter)))
                Bind(parameter.IsArray ? parameter.GetElementType() : parameter.GetGenericArguments()[0], list[0], generic, bound);
        }

        static int Depth(Type type)
        {
            int depth = 0;
            for (Type at = type; at != null; at = at.BaseType) depth++;
            return depth;
        }

        // How well the least fitting of these goes to the type.
        static int Worst(IEnumerable<object> items, Type want)
        {
            int worst = 0;
            foreach (object item in items)
            {
                int fit = Fit(item, want);
                if (fit < 0) return -1;
                worst = Math.Max(worst, fit);
            }
            return worst;
        }

        static bool IsListLike(Type type) =>
            type.IsGenericType && new[] { typeof(List<>), typeof(IList<>), typeof(IEnumerable<>), typeof(ICollection<>), typeof(IReadOnlyList<>), typeof(IReadOnlyCollection<>), typeof(HashSet<>), typeof(ISet<>) }.Contains(type.GetGenericTypeDefinition());

        static bool IsDictionaryLike(Type type) =>
            type.IsGenericType && new[] { typeof(Dictionary<,>), typeof(IDictionary<,>), typeof(IReadOnlyDictionary<,>) }.Contains(type.GetGenericTypeDefinition());

        // A value turned into the type of the parameter it is given to.
        static object Turn(object value, Type want)
        {
            if (want.IsByRef) want = want.GetElementType();
            if (value == null) return null;
            if (value is Named type && want.IsAssignableFrom(typeof(Type))) return type.Type;
            Type plain = Nullable.GetUnderlyingType(want) ?? want;
            if (plain.IsInstanceOfType(value)) return value;
            if (plain.IsEnum) return Enum.ToObject(plain, value);
            if (plain == typeof(char) && value is string s) return s[0];
            if (plain.IsPrimitive || plain == typeof(decimal)) return Convert.ChangeType(value, plain, CultureInfo.InvariantCulture);
            if (value is IList list)
            {
                Type item = plain.IsArray ? plain.GetElementType() : plain.GetGenericArguments()[0];
                if (plain.IsArray)
                {
                    Array made = Array.CreateInstance(item, list.Count);
                    for (int i = 0; i < list.Count; i++) made.SetValue(Turn(list[i], item), i);
                    return made;
                }
                Type kind = plain.GetGenericTypeDefinition();
                Type concrete = kind == typeof(HashSet<>) || kind == typeof(ISet<>) ? typeof(HashSet<>).MakeGenericType(item) : typeof(List<>).MakeGenericType(item);
                object collection = Activator.CreateInstance(concrete);
                MethodInfo add = concrete.GetMethod("Add");
                foreach (object one in list) add.Invoke(collection, new[] { Turn(one, item) });
                return collection;
            }
            if (value is IDictionary dictionary)
            {
                Type[] both = plain.GetGenericArguments();
                Type concrete = typeof(Dictionary<,>).MakeGenericType(both);
                var made = (IDictionary)Activator.CreateInstance(concrete);
                foreach (DictionaryEntry entry in dictionary) made[Turn(entry.Key, both[0])] = Turn(entry.Value, both[1]);
                return made;
            }
            return value;
        }

        static string Written(Type type)
        {
            if (type.IsByRef) return "ref " + Written(type.GetElementType());
            Type plain = Nullable.GetUnderlyingType(type);
            if (plain != null) return Written(plain) + "?";
            if (type.IsArray) return Written(type.GetElementType()) + "[]";
            if (type.IsGenericType)
            {
                string name = type.Name.Substring(0, type.Name.IndexOf('`'));
                return name + "<" + string.Join(", ", type.GetGenericArguments().Select(Written)) + ">";
            }
            string word = Words.FirstOrDefault(pair => pair.Value == type).Key;
            return word ?? type.Name;
        }

        static string Signature(MethodBase method) =>
            (method is ConstructorInfo ? Written(method.DeclaringType) : method.Name) + "(" + string.Join(", ", method.GetParameters().Select(p => Written(p.ParameterType) + " " + p.Name)) + ")";

        static string Kinds(object[] args) =>
            args.Length == 0 ? "nothing" : string.Join(", ", args.Select(a => a == null ? "none" : a is long ? "an int" : a is double ? "a float" : a is string ? "a str" : a is bool ? "a bool" : a is IList ? "a list" : a is IDictionary ? "a dict" : Described(a)));

        // The one of these methods that the arguments fit best. Optional
        // parameters may be left out, and `params` takes the rest. Two
        // that fit as well are an error and not a guess.
        static (MethodBase, object[]) Chosen(IEnumerable<MethodBase> methods, object[] args, string what) =>
            Chosen(methods, args, new Dictionary<string, object>(), what);

        // Named arguments are given to the parameters of those names, as
        // C# gives them.
        static (MethodBase, object[]) Chosen(IEnumerable<MethodBase> methods, object[] args, Dictionary<string, object> named, string what)
        {
            MethodBase best = null;
            object[] bestArgs = null;
            int bestScore = int.MaxValue;
            bool tie = false;
            // A method that hides one of its base type's (`new`) is the
            // one there is.
            var all = methods.Select(m => m.ContainsGenericParameters && m is MethodInfo generic ? Inferred(generic, args) : m)
                .Where(m => m != null && !m.ContainsGenericParameters)
                .GroupBy(m => string.Join(",", m.GetParameters().Select(p => p.ParameterType.ToString())))
                .Select(same => same.OrderByDescending(m => Depth(m.DeclaringType)).First())
                .ToList();
            foreach (MethodBase method in all)
            {
                ParameterInfo[] parameters = method.GetParameters();
                bool spread = named.Count == 0 && parameters.Length > 0 && parameters[parameters.Length - 1].IsDefined(typeof(ParamArrayAttribute), false);
                if (named.Keys.Any(name => !parameters.Skip(args.Length).Any(p => p.Name == name))) continue;
                var given = new object[parameters.Length];
                int score = 0;
                bool fits = true;
                for (int i = 0; i < parameters.Length && fits; i++)
                {
                    Type want = parameters[i].ParameterType;
                    if (spread && i == parameters.Length - 1)
                    {
                        Type item = want.GetElementType();
                        // An array given as itself, or the rest spread.
                        if (args.Length == parameters.Length && args[i] is IList && Fit(args[i], want) >= 0)
                        {
                            score += Fit(args[i], want);
                            given[i] = Turn(args[i], want);
                            break;
                        }
                        int rest = Math.Max(0, args.Length - i);
                        Array made = Array.CreateInstance(item, rest);
                        for (int j = 0; j < rest; j++)
                        {
                            int fit = Fit(args[i + j], item);
                            if (fit < 0) { fits = false; break; }
                            score += fit + 1;
                            made.SetValue(Turn(args[i + j], item), j);
                        }
                        given[i] = made;
                        score += 1;
                        break;
                    }
                    if (i >= args.Length && named.TryGetValue(parameters[i].Name, out object given_by_name))
                    {
                        int fit = Fit(given_by_name, want);
                        if (fit < 0) { fits = false; break; }
                        score += fit;
                        given[i] = Turn(given_by_name, want);
                        continue;
                    }
                    if (i >= args.Length)
                    {
                        if (!parameters[i].IsOptional) { fits = false; break; }
                        given[i] = parameters[i].DefaultValue is DBNull ? null : parameters[i].DefaultValue;
                        continue;
                    }
                    int f = Fit(args[i], want);
                    if (f < 0) { fits = false; break; }
                    score += f;
                    given[i] = Turn(args[i], want);
                }
                if (!spread && args.Length > parameters.Length) fits = false;
                if (!fits) continue;
                if (score < bestScore) { best = method; bestArgs = given; bestScore = score; tie = false; }
                else if (score == bestScore) tie = true;
            }
            if (best == null || tie)
            {
                string there = string.Join(", ", all.Select(Signature).OrderBy(s => s));
                throw new Told("Error", tie
                    ? what + " has more than one that " + Kinds(args) + " fit as well: " + there
                    : what + " has none that takes " + Kinds(args) + (named.Count == 0 ? "" : " and " + string.Join(", ", named.Keys.Select(name => "`" + name + "`"))) + ". There is " + there);
            }
            return (best, bestArgs);
        }

        // ------------------------------------------------- What Zeno asks for

        // The names C# has for the types it has words for.
        static readonly Dictionary<string, Type> Words = new Dictionary<string, Type>
        {
            ["int"] = typeof(int), ["long"] = typeof(long), ["short"] = typeof(short), ["byte"] = typeof(byte),
            ["sbyte"] = typeof(sbyte), ["uint"] = typeof(uint), ["ulong"] = typeof(ulong), ["ushort"] = typeof(ushort),
            ["double"] = typeof(double), ["float"] = typeof(float), ["decimal"] = typeof(decimal), ["bool"] = typeof(bool),
            ["char"] = typeof(char), ["string"] = typeof(string), ["object"] = typeof(object),
        };

        // `List<int>` and `Dictionary<string, List<int>>`, as C# writes a
        // type that is made of others, and `int[]`.
        static Type Generic(string name)
        {
            name = name.Trim();
            if (Words.TryGetValue(name, out Type word)) return word;
            if (name.EndsWith("[]")) return Generic(name.Substring(0, name.Length - 2))?.MakeArrayType();
            if (name.EndsWith("?")) { Type inner = Generic(name.Substring(0, name.Length - 1)); return inner == null ? null : typeof(Nullable<>).MakeGenericType(inner); }
            int open = name.IndexOf('<');
            if (open < 0 || !name.EndsWith(">")) return Find(name);
            var parts = new List<string>();
            int depth = 0, from = open + 1;
            for (int i = open + 1; i < name.Length - 1; i++)
            {
                if (name[i] == '<') depth++;
                else if (name[i] == '>') depth--;
                else if (name[i] == ',' && depth == 0) { parts.Add(name.Substring(from, i - from)); from = i + 1; }
            }
            parts.Add(name.Substring(from, name.Length - 1 - from));
            Type made = Find(name.Substring(0, open).Trim() + "`" + parts.Count);
            var inside = parts.Select(Generic).ToArray();
            return made == null || inside.Any(one => one == null) ? null : made.MakeGenericType(inside);
        }

        static Type Find(string name)
        {
            if (name.Contains('<') || name.EndsWith("[]") || name.EndsWith("?") || Words.ContainsKey(name)) return Generic(name);
            Type found = Type.GetType(name, false);
            if (found != null) return found;
            foreach (Assembly assembly in Loaded.Concat(AppDomain.CurrentDomain.GetAssemblies()))
            {
                found = assembly.GetType(name, false);
                if (found != null) return found;
            }
            // `System.Net.Http.HttpClient` is in an assembly that is not
            // loaded until something asks for it, and its name is most of
            // the name of the type.
            for (string part = name; part.Contains('.'); part = part.Substring(0, part.LastIndexOf('.')))
            {
                try
                {
                    found = Assembly.Load(new AssemblyName(part)).GetType(name, false);
                    if (found != null) return found;
                }
                catch (Exception) { }
            }
            // A package whose name is not the start of the name of the
            // type: each is looked in, once.
            foreach (string path in Package.Values.ToList())
            {
                Assembly assembly;
                try { assembly = Assembly.LoadFrom(path); }
                catch (Exception) { continue; }
                if (!Loaded.Contains(assembly)) Loaded.Add(assembly);
                found = assembly.GetType(name, false);
                if (found != null) return found;
            }
            // A type inside another is written with a dot, as C# writes it.
            int dot = name.LastIndexOf('.');
            found = dot > 0 ? Find(name.Substring(0, dot) + "+" + name.Substring(dot + 1)) : null;
            if (found != null || name.Contains('+')) return found;
            // A name with no namespace is looked for where a program of C#
            // looks without a `using`: the namespaces a new project has.
            if (!name.Contains('.'))
            {
                foreach (string space in Usings)
                {
                    found = Find(space + "." + name);
                    if (found != null) return found;
                }
            }
            return null;
        }

        static readonly string[] Usings =
        {
            "System", "System.Collections.Generic", "System.IO", "System.Linq", "System.Net.Http", "System.Threading", "System.Threading.Tasks",
        };

        static object TypeNamed(string name)
        {
            Type found;
            try { found = Find(name); }
            catch (ArgumentException error) { throw new Told("ValueError", "`" + name + "` cannot be made: " + error.Message); }
            if (found == null)
            {
                throw new Told("FileError", ".NET has no type `" + name + "`. A type of a package is there once the project beside the program has the package (`dotnet add package`), and one of a .dll after `csharp.assembly`");
            }
            return Keep(new Named(found));
        }

        // The packages of NuGet that the project beside the program has,
        // in its folder or one above: `dotnet add package` writes down in
        // obj/project.assets.json the .dll files of each package and of
        // every package it needs, and where NuGet keeps them.
        static void Packages()
        {
            for (string folder = Home; !string.IsNullOrEmpty(folder); folder = Path.GetDirectoryName(folder))
            {
                string file = Path.Combine(folder, "obj", "project.assets.json");
                if (!File.Exists(file)) continue;
                try
                {
                    using JsonDocument assets = JsonDocument.Parse(File.ReadAllText(file));
                    JsonElement root = assets.RootElement;
                    var places = root.GetProperty("packageFolders").EnumerateObject().Select(place => place.Name).ToList();
                    JsonElement libraries = root.GetProperty("libraries");
                    // The first target that is not for one system only.
                    JsonElement target = root.GetProperty("targets").EnumerateObject().First(one => !one.Name.Contains('/')).Value;
                    foreach (JsonProperty package in target.EnumerateObject())
                    {
                        if (!package.Value.TryGetProperty("runtime", out JsonElement runtime)) continue;
                        if (!libraries.TryGetProperty(package.Name, out JsonElement library) || !library.TryGetProperty("path", out JsonElement path)) continue;
                        foreach (JsonProperty dll in runtime.EnumerateObject())
                        {
                            if (!dll.Name.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)) continue;
                            string whole = places.Select(place => Path.Combine(place, path.GetString(), dll.Name)).FirstOrDefault(File.Exists);
                            if (whole != null) Package[Path.GetFileNameWithoutExtension(whole)] = whole;
                        }
                    }
                }
                catch (Exception) { }
                return;
            }
        }

        static Assembly Resolve(object sender, ResolveEventArgs asked)
        {
            if (Package.TryGetValue(new AssemblyName(asked.Name).Name, out string package)) return Assembly.LoadFrom(package);
            string file = new AssemblyName(asked.Name).Name + ".dll";
            foreach (string folder in Folders)
            {
                string path = Path.Combine(folder, file);
                if (File.Exists(path)) return Assembly.LoadFrom(path);
            }
            return null;
        }

        // `csharp.assembly("lib/Humanizer.dll")`, or a folder of them. A
        // path that does not begin at the top is from the folder of the
        // program.
        static object Assemblies(string path)
        {
            string whole = Path.IsPathRooted(path) || Home == "" ? Path.GetFullPath(path) : Path.GetFullPath(Path.Combine(Home, path));
            IEnumerable<string> files;
            if (Directory.Exists(whole)) files = Directory.GetFiles(whole, "*.dll").OrderBy(f => f);
            else if (File.Exists(whole)) files = new[] { whole };
            else throw new Told("FileError", "there is no .dll or folder `" + whole + "`");
            string folder = Directory.Exists(whole) ? whole : Path.GetDirectoryName(whole);
            if (!Folders.Contains(folder)) Folders.Add(folder);
            foreach (string file in files) Loaded.Add(Assembly.LoadFrom(file));
            return null;
        }

        const BindingFlags Public = BindingFlags.Public | BindingFlags.FlattenHierarchy;

        static object Get(object thing, string name)
        {
            bool isType = thing is Named;
            Type type = isType ? ((Named)thing).Type : thing.GetType();
            object on = isType ? null : thing;
            BindingFlags flags = Public | (isType ? BindingFlags.Static : BindingFlags.Instance);
            if (thing is Bound) throw new Told("Error", "a method has no `" + name + "`. In Zeno a call has brackets");
            PropertyInfo property = type.GetProperty(name, flags);
            if (property != null && property.GetIndexParameters().Length == 0 && property.CanRead) return Out(Settled(property.GetValue(on)));
            FieldInfo field = type.GetField(name, flags);
            if (field != null) return Out(field.GetValue(on));
            if (type.GetMethods(flags).Any(m => m.Name == name)) return Keep(new Bound(on, type, name));
            if (isType)
            {
                Type inner = type.GetNestedType(name, BindingFlags.Public);
                if (inner != null) return Keep(new Named(inner));
            }
            throw new Told("Error", Described(thing) + " has no " + (isType ? "static " : "") + "property, field or method `" + name + "`");
        }

        static void Set(object thing, string name, object to)
        {
            bool isType = thing is Named;
            Type type = isType ? ((Named)thing).Type : thing.GetType();
            object on = isType ? null : thing;
            BindingFlags flags = Public | (isType ? BindingFlags.Static : BindingFlags.Instance);
            PropertyInfo property = type.GetProperty(name, flags);
            if (property != null && property.CanWrite && property.GetSetMethod() != null)
            {
                if (Fit(to, property.PropertyType) < 0) throw new Told("Error", "`" + name + "` is " + A(Written(property.PropertyType)) + ", and this is " + Kinds(new[] { to }));
                property.SetValue(on, Turn(to, property.PropertyType));
                return;
            }
            FieldInfo field = type.GetField(name, flags);
            if (field != null && !field.IsInitOnly && !field.IsLiteral)
            {
                if (Fit(to, field.FieldType) < 0) throw new Told("Error", "`" + name + "` is " + A(Written(field.FieldType)) + ", and this is " + Kinds(new[] { to }));
                field.SetValue(on, Turn(to, field.FieldType));
                return;
            }
            throw new Told("Error", Described(thing) + " has no " + (isType ? "static " : "") + "property or field `" + name + "` that can be set");
        }

        // What takes time gives a Task, and the answer is waited for.
        static object Settled(object value)
        {
            if (value is Task task)
            {
                task.GetAwaiter().GetResult();
                Type type = task.GetType();
                PropertyInfo result = type.IsGenericType ? type.GetProperty("Result") : null;
                object got = result?.GetValue(task);
                return got != null && got.GetType().FullName == "System.Threading.Tasks.VoidTaskResult" ? null : got;
            }
            return value;
        }

        static object Call(object thing, object[] args, Dictionary<string, object> named)
        {
            if (thing is Named made)
            {
                Type type = made.Type;
                if (type.IsAbstract || type.IsInterface)
                    throw new Told("Error", "`" + type.Name + "` is " + (type.IsInterface ? "an interface" : type.IsSealed ? "a static class" : "abstract") + ", and no object can be made of it by calling it");
                var constructors = type.GetConstructors().Cast<MethodBase>().ToList();
                if (type.IsValueType && args.Length == 0 && named.Count == 0) return Activator.CreateInstance(type);
                if (constructors.Count == 0) throw new Told("Error", "`" + type.Name + "` has no constructor that may be used");
                var (constructor, given) = Chosen(constructors, args, named, "`" + Written(type) + "`");
                return ((ConstructorInfo)constructor).Invoke(given);
            }
            if (thing is Bound bound)
            {
                BindingFlags flags = Public | (bound.On == null ? BindingFlags.Static : BindingFlags.Instance);
                var methods = bound.Type.GetMethods(flags).Where(m => m.Name == bound.Name).Cast<MethodBase>();
                var (method, given) = Chosen(methods, args, named, "`" + bound.Name + "`");
                return Settled(method.Invoke(bound.On, given));
            }
            if (thing is Delegate function)
            {
                var (method, given) = Chosen(new MethodBase[] { function.Method.DeclaringType != null ? function.GetType().GetMethod("Invoke") : function.Method }, args, named, "this function");
                return Settled(function.DynamicInvoke(given));
            }
            throw new Told("Error", Described(thing) + " is not a method or a type, and cannot be called");
        }

        // `thing[index]`: an array's item, an indexer's (`list[0]`,
        // `dictionary["key"]`). On a type, and on an object that has no
        // items, the name in brackets is the name after a dot, for one
        // that is a word of Zeno.
        static object Item(object thing, object at)
        {
            if (thing is Array array)
            {
                if (!(at is long i) || i < 0 || i >= array.Length) throw new Told("IndexError", "index " + Shown(at) + " is out of range for " + array.Length + " items");
                return Out(array.GetValue(i));
            }
            if (thing is string || thing is Bound) throw new Told("Error", Described(thing) + " has no items");
            Type type = thing is Named named ? named.Type : thing.GetType();
            var indexers = thing is Named ? new List<MethodBase>() : type.GetProperties(Public | BindingFlags.Instance).Where(p => p.GetIndexParameters().Length == 1 && p.GetGetMethod() != null).Select(p => (MethodBase)p.GetGetMethod()).ToList();
            if (indexers.Count == 0 && at is string name) return Get(thing, name);
            if (indexers.Count == 0) throw new Told("Error", Described(thing) + " has no items");
            var (method, given) = Chosen(indexers, new[] { at }, "the items of " + Described(thing));
            return Out(method.Invoke(thing, given));
        }

        static void SetItem(object thing, object at, object to)
        {
            if (thing is Array array)
            {
                if (!(at is long i) || i < 0 || i >= array.Length) throw new Told("IndexError", "index " + Shown(at) + " is out of range for " + array.Length + " items");
                Type item = array.GetType().GetElementType();
                if (Fit(to, item) < 0) throw new Told("Error", "the items of this array are " + Written(item) + ", and this is " + Kinds(new[] { to }));
                array.SetValue(Turn(to, item), i);
                return;
            }
            var setters = thing.GetType().GetProperties(Public | BindingFlags.Instance).Where(p => p.GetIndexParameters().Length == 1 && p.GetSetMethod() != null).Select(p => (MethodBase)p.GetSetMethod()).ToList();
            if (setters.Count == 0 && at is string name) { Set(thing, name, to); return; }
            if (setters.Count == 0) throw new Told("Error", Described(thing) + " has no items that can be set");
            var (method, given) = Chosen(setters, new[] { at, to }, "the items of " + Described(thing));
            method.Invoke(thing, given);
        }

        static long Length(object thing)
        {
            if (thing is string s) return s.Length;
            if (thing is ICollection collection) return collection.Count;
            PropertyInfo count = thing.GetType().GetProperty("Count", BindingFlags.Public | BindingFlags.Instance) ?? thing.GetType().GetProperty("Length", BindingFlags.Public | BindingFlags.Instance);
            if (count != null && !(thing is Named) && IsWhole(count.GetValue(thing) ?? "")) return Convert.ToInt64(count.GetValue(thing));
            throw new Told("Error", Described(thing) + " has no length");
        }

        static bool Has(object thing, object item)
        {
            if (thing is IDictionary dictionary) return item != null && dictionary.Contains(Key(dictionary, item));
            MethodInfo containsKey = thing.GetType().GetMethod("ContainsKey");
            if (containsKey != null && containsKey.GetParameters().Length == 1 && Fit(item, containsKey.GetParameters()[0].ParameterType) >= 0)
                return (bool)containsKey.Invoke(thing, new[] { Turn(item, containsKey.GetParameters()[0].ParameterType) });
            if (thing is IEnumerable items && !(thing is string))
            {
                foreach (object one in items) if (Same(Out(one), item) || Equals(one, item)) return true;
                return false;
            }
            throw new Told("Error", Described(thing) + " cannot be asked what is `in` it");
        }

        static object Key(IDictionary dictionary, object item)
        {
            Type[] both = dictionary.GetType().IsGenericType ? dictionary.GetType().GetGenericArguments() : null;
            return both != null && both.Length == 2 && Fit(item, both[0]) >= 0 ? Turn(item, both[0]) : item;
        }

        static bool Same(object one, object other) => one is string || one is long || one is double || one is bool ? Equals(one, other) : false;

        static List<object> Items(object thing)
        {
            if (thing is IDictionary dictionary) return dictionary.Keys.Cast<object>().Select(Out).ToList();
            if (thing is IEnumerable items && !(thing is string) && !(thing is Named) && !(thing is Bound)) return items.Cast<object>().Select(Out).ToList();
            throw new Told("Error", Described(thing) + " cannot be gone through with `for`");
        }

        static string A(string name) => ("AEIOUaeiou".IndexOf(name[0]) >= 0 ? "an " : "a ") + name;

        static string Described(object thing)
        {
            if (thing is Named named) return "the " + (named.Type.IsInterface ? "interface " : named.Type.IsEnum ? "enum " : named.Type.IsValueType ? "struct " : "class ") + named.Type.FullName;
            if (thing is Bound) return "a method";
            string name = thing.GetType().IsGenericType || thing.GetType().IsArray ? Written(thing.GetType()) : thing.GetType().FullName;
            return A(name);
        }

        static string Shown(object thing)
        {
            if (thing == null) return "null";
            if (thing is Named named) return named.Type.FullName;
            if (thing is Bound bound) return "method `" + bound.Name + "` of " + (bound.On == null ? "the type " + bound.Type.FullName : Described(bound.On)) + ". In Zeno a call has brackets: `." + bound.Name + "()`";
            if (thing is string s) return s;
            if (thing is bool b) return b ? "true" : "false";
            if (thing is IFormattable formattable) return formattable.ToString(null, CultureInfo.InvariantCulture);
            if (thing is IEnumerable items && thing.GetType().ToString() == thing.ToString())
            {
                var parts = new List<string>();
                foreach (object one in items)
                {
                    if (parts.Count == 100) { parts.Add("..."); break; }
                    parts.Add(one is string text ? JsonSerializer.Serialize(text) : Shown(one));
                }
                return (thing is IDictionary ? "{" : "[") + string.Join(", ", parts) + (thing is IDictionary ? "}" : "]");
            }
            if (thing.GetType().IsGenericType && thing.GetType().GetGenericTypeDefinition() == typeof(KeyValuePair<,>))
            {
                object key = thing.GetType().GetProperty("Key").GetValue(thing);
                object value = thing.GetType().GetProperty("Value").GetValue(thing);
                return (key is string k ? JsonSerializer.Serialize(k) : Shown(key)) + ": " + (value is string v ? JsonSerializer.Serialize(v) : Shown(value));
            }
            return thing.ToString();
        }

        static object Act(JsonElement ask)
        {
            string op = ask.GetProperty("op").GetString();
            string name = ask.TryGetProperty("name", out JsonElement n) ? n.GetString() : null;
            switch (op)
            {
                case "import": return TypeNamed(name);
                case "jar": return Assemblies(name);
            }
            object thing = Back(ask.GetProperty("on"));
            switch (op)
            {
                case "get": return Get(thing, name);
                case "set": Set(thing, name, Back(ask.GetProperty("to"))); return null;
                case "call":
                    object[] args = ask.GetProperty("args").EnumerateArray().Select(Back).ToArray();
                    var named = new Dictionary<string, object>();
                    foreach (JsonElement pair in ask.GetProperty("named").EnumerateArray()) named[pair[0].GetString()] = Back(pair[1]);
                    return Out(Call(thing, args, named));
                case "item": return Item(thing, Back(ask.GetProperty("at")));
                case "set_item": SetItem(thing, Back(ask.GetProperty("at")), Back(ask.GetProperty("to"))); return null;
                case "len": return Length(thing);
                case "has": return Has(thing, Back(ask.GetProperty("item")));
                case "items": return new object[] { "l", Items(thing) };
                case "copy": return Copied(thing);
                case "str":
                case "repr":
                    string text = Shown(thing);
                    return IsText(text) ? text : thing.GetType().FullName;
                case "equal":
                    object other = Back(ask.GetProperty("to"));
                    return thing is Named a && other is Named b ? a.Type == b.Type : Equals(thing, other);
            }
            throw new Told("Error", "Zeno asked for `" + op + "`, which this .NET end does not know");
        }

        // The kind of error Zeno raises for an exception of .NET: its own
        // kind where the two mean one thing, and `Error` for the rest.
        static string KindOf(Exception error)
        {
            switch (error)
            {
                case Told told: return told.Kind;
                case DivideByZeroException _: return "ZeroDivisionError";
                case KeyNotFoundException _: return "KeyError";
                case IndexOutOfRangeException _: return "IndexError";
                case ArgumentOutOfRangeException _: return "IndexError";
                case ArgumentException _: return "ValueError";
                case FormatException _: return "ValueError";
                case OverflowException _: return "OverflowError";
                case IOException _: return "FileError";
                case UnauthorizedAccessException _: return "FileError";
            }
            return "Error";
        }

        static string Said(Exception error)
        {
            if (error is Told) return error.Message;
            string name = error.GetType().Name;
            return error.Message == "" ? name : name + ": " + error.Message;
        }

        // ------------------------------------------------- The lines

        static void Write(Utf8JsonWriter writer, object value)
        {
            switch (value)
            {
                case null: writer.WriteNullValue(); break;
                case bool b: writer.WriteBooleanValue(b); break;
                case long l: writer.WriteNumberValue(l); break;
                case double d:
                    // A float always has a point, so that it arrives as a float.
                    string text = d.ToString("R", CultureInfo.InvariantCulture);
                    if (!text.Contains('.') && !text.Contains('E')) text += ".0";
                    writer.WriteRawValue(text.Replace("E", "e"));
                    break;
                case string s: writer.WriteStringValue(s); break;
                case IEnumerable items:
                    writer.WriteStartArray();
                    foreach (object item in items) Write(writer, item);
                    writer.WriteEndArray();
                    break;
                default: writer.WriteStringValue(value.ToString()); break;
            }
        }

        static string Line(object answer, bool failed)
        {
            using var buffer = new MemoryStream();
            using (var writer = new Utf8JsonWriter(buffer, new JsonWriterOptions { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping }))
            {
                writer.WriteStartObject();
                writer.WritePropertyName(failed ? "err" : "ok");
                Write(writer, answer);
                writer.WriteEndObject();
            }
            return Encoding.UTF8.GetString(buffer.ToArray()) + "\n";
        }

        static void Main()
        {
            string[] said = (Environment.GetEnvironmentVariable("ZENO_BRIDGE") ?? "").Split(' ');
            Environment.SetEnvironmentVariable("ZENO_BRIDGE", null);
            Count = long.Parse(said[2], CultureInfo.InvariantCulture);
            Home = Environment.GetEnvironmentVariable("ZENO_PROGRAM") ?? "";
            Environment.SetEnvironmentVariable("ZENO_PROGRAM", null);
            CultureInfo.DefaultThreadCurrentCulture = CultureInfo.InvariantCulture;
            CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
            Packages();
            AppDomain.CurrentDomain.AssemblyResolve += Resolve;
            var client = new TcpClient("127.0.0.1", int.Parse(said[0], CultureInfo.InvariantCulture)) { NoDelay = true };
            NetworkStream stream = client.GetStream();
            var reader = new StreamReader(stream, new UTF8Encoding(false));
            var writer = new StreamWriter(stream, new UTF8Encoding(false)) { AutoFlush = true, NewLine = "\n" };
            writer.Write("{\"hello\":" + JsonSerializer.Serialize(said[1]) + ",\"version\":" + JsonSerializer.Serialize(Environment.Version.ToString()) + "}\n");
            string line;
            while ((line = reader.ReadLine()) != null)
            {
                string answer;
                try
                {
                    using JsonDocument document = JsonDocument.Parse(line, new JsonDocumentOptions { MaxDepth = 1000 });
                    JsonElement ask = document.RootElement;
                    if (ask.TryGetProperty("free", out JsonElement free))
                        foreach (JsonElement number in free.EnumerateArray()) Kept.Remove(number.GetInt64());
                    answer = Line(Act(ask), false);
                }
                catch (Exception thrown)
                {
                    Exception error = thrown;
                    while ((error is TargetInvocationException || error is AggregateException) && error.InnerException != null) error = error.InnerException;
                    answer = Line(new object[] { KindOf(error), Said(error) }, true);
                }
                // What .NET printed is on the screen before Zeno goes on.
                Console.Out.Flush();
                Console.Error.Flush();
                writer.Write(answer);
            }
        }
    }
}
