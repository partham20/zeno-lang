<?php
// The PHP end of `import php` (docs/php.md).
//
// `zeno run` starts PHP with this file as its program. It connects
// back to Zeno over a socket of this machine, then does what each line
// it is sent asks for, and answers with one line. The lines are the
// ones bridge.py reads and writes:
//
//     none, a bool, an int, a float, a str     as JSON writes them
//     a float JSON cannot write                ["f", "nan"], "inf", "-inf"
//     a list                                   ["l", [item, ...]]
//     a dict                                   ["d", [[key, value], ...]]
//     anything else, which stays here          ["p", number]
//
// An array of PHP is a value there and not an object, so it goes to
// Zeno as a list or a dict, whole. An object stays here, in $KEPT
// under its number, until Zeno says that nobody there holds it any
// more.

ini_set('serialize_precision', '-1');

$KEPT = [];
$COUNT = 0;
$HOME = '';

// Something this end has to say itself, as an error of the kind Zeno
// knows it by.
final class ZenoTold extends Exception
{
    public $kind;

    public function __construct($kind, $text)
    {
        parent::__construct($text);
        $this->kind = $kind;
    }
}

// A warning of PHP, raised: `file_get_contents` of a file that is not
// there gives `false` and a line on the screen, and a program of Zeno
// would go on with the `false`.
final class ZenoWarned extends ErrorException
{
}

// `php.type("DateTime")`: a class by its name. PHP has no value for
// that, so this is one.
final class ZenoType
{
    public $name;

    public function __construct($name)
    {
        $this->name = $name;
    }
}

// `php.callable("strrev")`: a function by its name.
final class ZenoFunction
{
    public $name;

    public function __construct($name)
    {
        $this->name = $name;
    }
}

// `thing.name`: a method and what it will be called on (an object, or
// the name of a class for a static one), kept until it is called.
final class ZenoBound
{
    public $on;
    public $name;

    public function __construct($on, $name)
    {
        $this->on = $on;
        $this->name = $name;
    }
}

// `php.variable(value)`: a variable of PHP, for a function that
// changes what it is given (`sort(&$array)`).
final class ZenoVariable
{
    public $value;

    public function __construct($value)
    {
        $this->value = $value;
    }
}

function zeno_keep($thing)
{
    global $KEPT, $COUNT;
    $COUNT += 1;
    $KEPT[$COUNT] = $thing;
    return ['p', $COUNT];
}

// A string of PHP is bytes. It is text where they read as UTF-8.
function zeno_is_text($bytes)
{
    return preg_match('//u', $bytes) === 1;
}

// An array whose keys are 0, 1, 2 and so on is a list. Any other is a
// dict. An empty one is a list.
function zeno_is_listed(array $items)
{
    if (function_exists('array_is_list')) {
        return array_is_list($items);
    }
    $next = 0;
    foreach ($items as $key => $_) {
        if ($key !== $next) {
            return false;
        }
        $next += 1;
    }
    return true;
}

// A value of PHP, as it goes to Zeno. null, a bool, an int, a float,
// a string of text and an array go as themselves. Everything else
// stays here.
function zeno_out($value, $depth = 0)
{
    if ($value === null || is_bool($value) || is_int($value)) {
        return $value;
    }
    if (is_float($value)) {
        if (is_nan($value)) {
            return ['f', 'nan'];
        }
        if (is_infinite($value)) {
            return ['f', $value > 0 ? 'inf' : '-inf'];
        }
        return $value;
    }
    if (is_string($value)) {
        return zeno_is_text($value) ? $value : zeno_keep($value);
    }
    if (is_array($value)) {
        if ($depth > 200) {
            throw new ZenoTold('ValueError', 'this is nested more than 200 deep, or holds itself, and cannot be given to Zeno');
        }
        if (zeno_is_listed($value)) {
            $items = [];
            foreach ($value as $item) {
                $items[] = zeno_out($item, $depth + 1);
            }
            return ['l', $items];
        }
        $pairs = [];
        foreach ($value as $key => $item) {
            if (is_string($key) && !zeno_is_text($key)) {
                throw new ZenoTold('ValueError', 'a dict in Zeno has keys that are str or int, and this array has a key that is not text: ' . zeno_shown($key, 'repr'));
            }
            $pairs[] = [$key, zeno_out($item, $depth + 1)];
        }
        return ['d', $pairs];
    }
    return zeno_keep($value);
}

// `php.copy`: what can be gone through, and an object that only holds
// values (what `json_decode` makes), as a list or a dict, with
// everything in it.
function zeno_copied($value, $depth = 0)
{
    if ($depth > 200) {
        throw new ZenoTold('ValueError', 'this is nested more than 200 deep, or holds itself, and cannot be copied');
    }
    if ($value instanceof ZenoVariable) {
        return zeno_copied($value->value, $depth + 1);
    }
    if ($value instanceof stdClass) {
        $value = get_object_vars($value);
        if ($value === []) {
            return ['d', []];
        }
    } elseif ($value instanceof Traversable) {
        $made = [];
        $listed = true;
        foreach ($value as $key => $item) {
            if (!is_int($key) && !(is_string($key) && zeno_is_text($key))) {
                throw new ZenoTold('ValueError', 'a dict in Zeno has keys that are str or int, and this one has the key ' . zeno_shown($key, 'repr'));
            }
            $listed = $listed && $key === count($made);
            $made[] = [$key, $item];
        }
        if ($listed) {
            return ['l', array_map(function ($pair) use ($depth) {
                return zeno_copied($pair[1], $depth + 1);
            }, $made)];
        }
        return ['d', array_map(function ($pair) use ($depth) {
            return [$pair[0], zeno_copied($pair[1], $depth + 1)];
        }, $made)];
    }
    if (is_array($value)) {
        if (zeno_is_listed($value)) {
            $items = [];
            foreach ($value as $item) {
                $items[] = zeno_copied($item, $depth + 1);
            }
            return ['l', $items];
        }
        $pairs = [];
        foreach ($value as $key => $item) {
            if (is_string($key) && !zeno_is_text($key)) {
                throw new ZenoTold('ValueError', 'a dict in Zeno has keys that are str or int, and this array has a key that is not text: ' . zeno_shown($key, 'repr'));
            }
            $pairs[] = [$key, zeno_copied($item, $depth + 1)];
        }
        return ['d', $pairs];
    }
    return zeno_out($value);
}

// A value Zeno sent, as a value of PHP.
function zeno_back($value)
{
    global $KEPT;
    if (!is_array($value)) {
        return $value;
    }
    $tag = $value[0];
    $inside = $value[1];
    switch ($tag) {
        case 'p':
            if (!array_key_exists($inside, $KEPT)) {
                throw new ZenoTold('Error', 'this value was kept by a PHP that has stopped since');
            }
            return $KEPT[$inside];
        case 'l':
            return array_map('zeno_back', $inside);
        case 'd':
            $made = [];
            foreach ($inside as $pair) {
                $made[zeno_back($pair[0])] = zeno_back($pair[1]);
            }
            return $made;
        default:
            return ['nan' => NAN, 'inf' => INF, '-inf' => -INF][$inside];
    }
}

// The kind of error Zeno raises for an error of PHP: its own kind
// where the two mean one thing, and `Error` for the rest.
function zeno_kind_of($error)
{
    if ($error instanceof ZenoTold) {
        return $error->kind;
    }
    if ($error instanceof DivisionByZeroError) {
        return 'ZeroDivisionError';
    }
    if (class_exists('ValueError', false) && $error instanceof ValueError) {
        return 'ValueError';
    }
    if ($error instanceof InvalidArgumentException || $error instanceof DomainException || $error instanceof JsonException) {
        return 'ValueError';
    }
    if ($error instanceof OutOfBoundsException || $error instanceof OutOfRangeException) {
        return 'IndexError';
    }
    // PHP has no exception of its own for a file. A warning, and what
    // the classes for files throw, say what the system said.
    if (($error instanceof ZenoWarned || $error instanceof RuntimeException)
        && preg_match('/failed to open|No such file or directory|Permission denied|File exists|Is a directory|Not a directory|stat failed/i', $error->getMessage())) {
        return 'FileError';
    }
    return 'Error';
}

function zeno_said($error)
{
    $text = $error->getMessage();
    if ($error instanceof ZenoTold) {
        return $text;
    }
    $name = get_class($error);
    // `Exception` and `Error` are the plain ones, and a warning was
    // never an exception: the text is all there is to say.
    if ($text !== '' && ($name === zeno_kind_of($error) || in_array($name, ['Exception', 'Error', 'ErrorException', 'ZenoWarned'], true))) {
        return $text;
    }
    return $text === '' || $text === $name ? $name : $name . ': ' . $text;
}

function zeno_a($name)
{
    return (preg_match('/^[AEIOU]/i', $name) ? 'an ' : 'a ') . $name;
}

function zeno_described($thing)
{
    if ($thing instanceof ZenoType) {
        return 'the class ' . $thing->name;
    }
    if ($thing instanceof ZenoFunction) {
        return 'the function ' . $thing->name;
    }
    if ($thing instanceof ZenoBound) {
        return 'a method';
    }
    if ($thing instanceof ZenoVariable) {
        return 'a variable';
    }
    if ($thing instanceof Closure) {
        return 'a function';
    }
    if (is_object($thing)) {
        $name = get_class($thing);
        return strpos($name, '@anonymous') === false ? zeno_a($name) : 'an object of a class with no name';
    }
    if (is_string($thing)) {
        return zeno_is_text($thing) ? 'a str' : 'bytes';
    }
    if (is_array($thing)) {
        return zeno_is_listed($thing) ? 'a list' : 'a dict';
    }
    if ($thing === null) {
        return 'none';
    }
    if (is_bool($thing) || is_int($thing) || is_float($thing)) {
        return is_bool($thing) ? 'a bool' : (is_int($thing) ? 'an int' : 'a float');
    }
    if (is_resource($thing)) {
        return zeno_a(get_resource_type($thing)) . ' resource';
    }
    return zeno_a(gettype($thing));
}

// `php.type("DateTime")`
function zeno_type_named($name)
{
    global $HOME;
    $name = ltrim($name, '\\');
    $there = $name !== '' && (class_exists($name) || interface_exists($name) || trait_exists($name));
    if (!$there) {
        throw new ZenoTold(
            'FileError',
            'PHP has no class `' . $name . '`. Is its package installed in this folder (`composer require`)? This folder is ' . ($HOME === '' ? getcwd() : $HOME)
        );
    }
    return zeno_keep(new ZenoType((new ReflectionClass($name))->getName()));
}

// `php.callable("strrev")`, `php.callable("DateTime::createFromFormat")`
function zeno_function_named($name)
{
    $name = ltrim($name, '\\');
    if (in_array(strtolower($name), ['echo', 'print', 'isset', 'empty', 'unset', 'include', 'include_once', 'require', 'require_once', 'exit', 'die', 'eval', 'list', 'array'], true)) {
        throw new ZenoTold('Error', '`' . $name . '` is part of the language of PHP and not a function, so it cannot be called from Zeno');
    }
    if (!is_callable($name)) {
        throw new ZenoTold('Error', 'PHP has no function `' . $name . '`. One that an extension or a package brings is there when that is installed');
    }
    return zeno_keep(new ZenoFunction($name));
}

function zeno_constant_named($name)
{
    $name = ltrim($name, '\\');
    if (!defined($name)) {
        throw new ZenoTold('Error', 'PHP has no constant `' . $name . '`');
    }
    return zeno_out(constant($name));
}

// `php.require("helper.php")`: a file of PHP, read once. A path that
// does not begin at the top is from the folder of the program.
function zeno_required($path)
{
    global $HOME;
    $whole = $path;
    $begins = $path !== '' && ($path[0] === '/' || $path[0] === '\\' || preg_match('/^[A-Za-z]:/', $path));
    if (!$begins && $HOME !== '') {
        $whole = $HOME . DIRECTORY_SEPARATOR . $path;
    }
    if (!is_file($whole)) {
        throw new ZenoTold('FileError', 'there is no file `' . $whole . '`');
    }
    (static function ($file) {
        require_once $file;
    })($whole);
    return null;
}

function zeno_get($thing, $name)
{
    if ($thing instanceof ZenoType) {
        $class = $thing->name;
        if (defined($class . '::' . $name)) {
            return zeno_out(constant($class . '::' . $name));
        }
        if (property_exists($class, $name)) {
            $property = new ReflectionProperty($class, $name);
            if ($property->isStatic() && $property->isPublic()) {
                return zeno_out($property->getValue());
            }
        }
        if (method_exists($class, $name) || method_exists($class, '__callStatic')) {
            return zeno_keep(new ZenoBound($class, $name));
        }
        throw new ZenoTold('Error', zeno_described($thing) . ' has no constant, static property or static method `' . $name . '`');
    }
    if (is_object($thing) && !($thing instanceof ZenoFunction) && !($thing instanceof ZenoBound)) {
        // PHP keeps methods and properties apart, and a dot in Zeno is
        // both. A name that is a method is the method.
        if (method_exists($thing, $name) && is_callable([$thing, $name])) {
            return zeno_keep(new ZenoBound($thing, $name));
        }
        if (property_exists($thing, $name) || isset($thing->$name)) {
            return zeno_out(zeno_property($thing, $name));
        }
        if (method_exists($thing, '__call')) {
            return zeno_keep(new ZenoBound($thing, $name));
        }
        if (method_exists($thing, '__get')) {
            return zeno_out($thing->$name);
        }
    }
    throw new ZenoTold('Error', zeno_described($thing) . ' has no property or method `' . $name . '`');
}

function zeno_property($thing, $name)
{
    if (property_exists($thing, $name) && !isset($thing->$name) && !method_exists($thing, '__get')) {
        $property = new ReflectionProperty($thing, $name);
        if (!$property->isPublic()) {
            throw new ZenoTold('Error', 'the property `' . $name . '` of ' . zeno_described($thing) . ' is not public');
        }
        if (method_exists($property, 'isInitialized') && !$property->isInitialized($thing)) {
            throw new ZenoTold('Error', 'the property `' . $name . '` of ' . zeno_described($thing) . ' has no value yet');
        }
    }
    return $thing->$name;
}

// What Zeno gave a call, as PHP takes it. A variable is given as the
// variable, so that the function can change it.
function zeno_given(array $sent)
{
    $args = [];
    foreach ($sent as $at => $one) {
        $value = zeno_back($one);
        if ($value instanceof ZenoVariable) {
            $args[$at] = &$value->value;
        } else {
            $args[$at] = $value;
        }
        unset($value);
    }
    return $args;
}

// A function that changes what it is given (`&$array`) has to be
// given a variable. Given a list of Zeno it would change a copy that
// nobody sees, and say nothing.
function zeno_not_lost($takes, array $sent, $name)
{
    global $KEPT;
    foreach ($takes->getParameters() as $at => $parameter) {
        if (!$parameter->isPassedByReference() || $parameter->canBePassedByValue()) {
            continue;
        }
        $by = array_key_exists($at, $sent) ? $at : $parameter->getName();
        if (!array_key_exists($by, $sent)) {
            continue;
        }
        $one = $sent[$by];
        if (!(is_array($one) && $one[0] === 'p' && isset($KEPT[$one[1]]) && $KEPT[$one[1]] instanceof ZenoVariable)) {
            throw new ZenoTold(
                'Error',
                '`' . $name . '` changes its `' . $parameter->getName() . '` (PHP writes `&$' . $parameter->getName() . '`), so it needs a variable of PHP to change: `php.variable(value)`'
            );
        }
    }
}

function zeno_call($thing, array $sent, array $sent_named)
{
    if ($sent_named !== [] && PHP_MAJOR_VERSION < 8) {
        throw new ZenoTold('Error', 'named arguments need PHP 8, and this is PHP ' . PHP_VERSION);
    }
    $named = [];
    foreach ($sent_named as $pair) {
        $named[$pair[0]] = $pair[1];
    }
    if ($thing instanceof ZenoType) {
        $class = new ReflectionClass($thing->name);
        if (!$class->isInstantiable()) {
            $why = $class->isInterface() ? 'an interface' : ($class->isTrait() ? 'a trait' : ($class->isAbstract() ? 'abstract' : (method_exists($class, 'isEnum') && $class->isEnum() ? 'an enum' : 'a class that only PHP itself makes objects of')));
            throw new ZenoTold('Error', '`' . $thing->name . '` is ' . $why . ', and no object can be made of it by calling it');
        }
        if ($class->getConstructor() !== null) {
            zeno_not_lost($class->getConstructor(), $sent + $named, $thing->name);
        }
        $name = $thing->name;
        $args = zeno_given($sent);
        $more = zeno_given($named);
        return $more === [] ? new $name(...$args) : new $name(...$args, ...$more);
    }
    if ($thing instanceof ZenoFunction) {
        $target = $thing->name;
        $shown = $thing->name;
        try {
            $takes = strpos($target, '::') === false ? new ReflectionFunction($target) : new ReflectionMethod(...explode('::', $target, 2));
        } catch (ReflectionException $error) {
            $takes = null;
        }
    } elseif ($thing instanceof ZenoBound) {
        $target = [$thing->on, $thing->name];
        $shown = $thing->name;
        try {
            $takes = new ReflectionMethod($thing->on, $thing->name);
        } catch (ReflectionException $error) {
            $takes = null;
        }
    } elseif (is_object($thing) && is_callable($thing)) {
        $target = $thing;
        $shown = 'this function';
        $takes = null;
    } else {
        throw new ZenoTold('Error', zeno_described($thing) . ' is not a function, and cannot be called');
    }
    if ($takes !== null) {
        zeno_not_lost($takes, $sent + $named, $shown);
    }
    $args = zeno_given($sent);
    $more = zeno_given($named);
    return $more === [] ? $target(...$args) : $target(...$args, ...$more);
}

function zeno_item($thing, $at)
{
    if ($thing instanceof ArrayAccess) {
        return $thing[$at];
    }
    if (is_string($thing)) {
        if (!is_int($at) || $at < 0 || $at >= strlen($thing)) {
            throw new ZenoTold('IndexError', 'there is no byte ' . zeno_shown($at, 'repr') . ' in ' . strlen($thing) . ' bytes');
        }
        return ord($thing[$at]);
    }
    // A class, or an object with no items of its own: the name in
    // brackets is the name after a dot, for a name that is a word of
    // Zeno (`Size["from"]`). On an object it is the property first,
    // where the dot is the method first, so both can be reached when
    // one name is both.
    if ($thing instanceof ZenoType && is_string($at)) {
        return ['kept' => zeno_get($thing, $at)];
    }
    if (is_object($thing) && is_string($at) && !($thing instanceof ZenoFunction) && !($thing instanceof ZenoBound)) {
        if (property_exists($thing, $at) || isset($thing->$at)) {
            return zeno_property($thing, $at);
        }
        return ['kept' => zeno_get($thing, $at)];
    }
    throw new ZenoTold('Error', zeno_described($thing) . ' has no items');
}

function zeno_set_item($thing, $at, $to)
{
    if ($thing instanceof ArrayAccess) {
        $thing[$at] = $to;
        return;
    }
    if (is_object($thing) && is_string($at) && !($thing instanceof ZenoType) && !($thing instanceof ZenoFunction) && !($thing instanceof ZenoBound)) {
        $thing->$at = $to;
        return;
    }
    throw new ZenoTold('Error', zeno_described($thing) . ' has no items that can be set');
}

function zeno_length_of($thing)
{
    if ($thing instanceof Countable) {
        return count($thing);
    }
    if (is_string($thing)) {
        return strlen($thing);
    }
    throw new ZenoTold('Error', zeno_described($thing) . ' has no length');
}

function zeno_same($one, $other)
{
    if (is_object($one) && is_object($other)) {
        try {
            return $one == $other;
        } catch (Throwable $error) {
            return $one === $other;
        }
    }
    return $one === $other;
}

function zeno_has($thing, $item)
{
    if ($thing instanceof ArrayAccess && (is_int($item) || is_string($item))) {
        return $thing->offsetExists($item);
    }
    if ($thing instanceof Traversable) {
        foreach ($thing as $one) {
            if (zeno_same($one, $item)) {
                return true;
            }
        }
        return false;
    }
    throw new ZenoTold('Error', zeno_described($thing) . ' cannot be asked what is `in` it');
}

// Whatever PHP can go through with `foreach`, taken now, all of it.
function zeno_items_of($thing)
{
    if ($thing instanceof Traversable) {
        $items = [];
        foreach ($thing as $item) {
            $items[] = zeno_out($item);
        }
        return $items;
    }
    if (is_string($thing)) {
        return array_values(unpack('C*', $thing));
    }
    throw new ZenoTold('Error', zeno_described($thing) . ' cannot be gone through with `for`');
}

function zeno_shown($thing, $how)
{
    if ($thing instanceof ZenoType) {
        return $how === 'repr' ? zeno_described($thing) : $thing->name;
    }
    if ($thing instanceof ZenoFunction) {
        return zeno_described($thing);
    }
    if ($thing instanceof ZenoBound) {
        return 'the method `' . $thing->name . '` of ' . (is_string($thing->on) ? 'the class ' . $thing->on : zeno_described($thing->on));
    }
    if ($thing instanceof ZenoVariable) {
        return 'a variable of PHP that holds ' . zeno_described($thing->value);
    }
    if (is_string($thing)) {
        $written = preg_replace_callback('/[^\x20-\x7E]|["\\\\]/', function ($found) {
            return $found[0] === '"' || $found[0] === '\\' ? '\\' . $found[0] : sprintf('\\x%02X', ord($found[0]));
        }, $thing);
        return zeno_is_text($thing) && $how !== 'repr' ? $thing : '"' . $written . '"';
    }
    if (interface_exists('UnitEnum', false) && $thing instanceof UnitEnum) {
        return get_class($thing) . '::' . $thing->name;
    }
    if (is_object($thing) && method_exists($thing, '__toString')) {
        return (string) $thing;
    }
    if (is_object($thing) && !($thing instanceof Closure)) {
        $inside = json_encode($thing, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES | JSON_PRESERVE_ZERO_FRACTION | JSON_PARTIAL_OUTPUT_ON_ERROR);
        $name = strpos(get_class($thing), '@anonymous') === false ? get_class($thing) : 'object';
        return in_array($inside, [false, '{}', '[]', 'null'], true) ? $name : $name . ' ' . $inside;
    }
    if (is_scalar($thing) || $thing === null) {
        return json_encode($thing, JSON_PRESERVE_ZERO_FRACTION);
    }
    return zeno_described($thing);
}

function zeno_act(array $ask)
{
    $op = $ask['op'];
    $name = isset($ask['name']) ? $ask['name'] : null;
    switch ($op) {
        case 'import':
            return zeno_type_named($name);
        case 'function':
            return zeno_function_named($name);
        case 'constant':
            return zeno_constant_named($name);
        case 'require':
            return zeno_required($name);
        case 'variable':
            return zeno_keep(new ZenoVariable(zeno_back($ask['value'])));
    }
    $thing = zeno_back($ask['on']);
    switch ($op) {
        case 'get':
            return zeno_get($thing, $name);
        case 'set':
            if (!is_object($thing) || $thing instanceof ZenoFunction || $thing instanceof ZenoBound) {
                throw new ZenoTold('Error', zeno_described($thing) . ' has no properties that can be set');
            }
            if ($thing instanceof ZenoType) {
                $property = property_exists($thing->name, $name) ? new ReflectionProperty($thing->name, $name) : null;
                if ($property === null || !$property->isStatic() || !$property->isPublic()) {
                    throw new ZenoTold('Error', zeno_described($thing) . ' has no static property `' . $name . '` that can be set');
                }
                $property->setValue(null, zeno_back($ask['to']));
                return null;
            }
            $thing->$name = zeno_back($ask['to']);
            return null;
        case 'call':
            return zeno_out(zeno_call($thing, $ask['args'], $ask['named']));
        case 'item':
            $found = zeno_item($thing, zeno_back($ask['at']));
            return is_array($found) && array_key_exists('kept', $found) ? $found['kept'] : zeno_out($found);
        case 'set_item':
            zeno_set_item($thing, zeno_back($ask['at']), zeno_back($ask['to']));
            return null;
        case 'len':
            return zeno_length_of($thing);
        case 'has':
            return zeno_has($thing, zeno_back($ask['item']));
        case 'items':
            return ['l', zeno_items_of($thing)];
        case 'copy':
            return zeno_copied($thing);
        case 'str':
        case 'repr':
            $text = zeno_shown($thing, $op);
            return zeno_is_text($text) ? $text : zeno_shown($text, 'repr');
        case 'equal':
            return zeno_same($thing, zeno_back($ask['to']));
    }
    throw new ZenoTold('Error', 'Zeno asked for `' . $op . '`, which this PHP end does not know');
}

function zeno_main()
{
    global $KEPT, $COUNT, $HOME;
    $said = explode(' ', (string) getenv('ZENO_BRIDGE'));
    putenv('ZENO_BRIDGE');
    $COUNT = (int) $said[2];
    // A package is looked for where Composer put it for a program of
    // PHP in the same place: `vendor` in the folder the program is
    // in, then in each folder above. A file is looked for from there
    // too.
    $HOME = (string) getenv('ZENO_PROGRAM');
    putenv('ZENO_PROGRAM');
    if ($HOME !== '') {
        set_include_path($HOME . PATH_SEPARATOR . get_include_path());
        for ($folder = $HOME; ; $folder = dirname($folder)) {
            if (is_file($folder . '/vendor/autoload.php')) {
                require_once $folder . '/vendor/autoload.php';
                break;
            }
            if (dirname($folder) === $folder) {
                break;
            }
        }
    }
    set_error_handler(function ($severity, $text, $file, $line) {
        // `@`, and what the program of PHP chose not to hear of.
        if (!(error_reporting() & $severity)) {
            return false;
        }
        // What only advises is left to PHP, which prints it.
        if ($severity & (E_DEPRECATED | E_USER_DEPRECATED | E_NOTICE | E_USER_NOTICE)) {
            return false;
        }
        throw new ZenoWarned($text, 0, $severity, $file, $line);
    });
    $link = @stream_socket_client(
        'tcp://127.0.0.1:' . (int) $said[0],
        $number,
        $why,
        30,
        STREAM_CLIENT_CONNECT,
        stream_context_create(['socket' => ['tcp_nodelay' => true]])
    );
    if ($link === false) {
        return;
    }
    $flags = JSON_PRESERVE_ZERO_FRACTION | JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE;
    $say = function ($answer) use ($link, $flags) {
        $line = json_encode($answer, $flags, 4096) . "\n";
        for ($sent = 0; $sent < strlen($line); $sent += $wrote) {
            $wrote = @fwrite($link, substr($line, $sent));
            if ($wrote === false || $wrote === 0) {
                exit(0);
            }
        }
    };
    $say(['hello' => $said[1], 'version' => PHP_VERSION, 'program' => PHP_BINARY]);
    while (($line = fgets($link)) !== false) {
        try {
            $ask = json_decode($line, true, 4096);
            foreach (isset($ask['free']) ? $ask['free'] : [] as $number) {
                unset($KEPT[$number]);
            }
            $answer = ['ok' => zeno_act($ask)];
        } catch (Throwable $error) {
            $answer = ['err' => [zeno_kind_of($error), zeno_said($error)]];
        }
        // What PHP printed is on the screen before Zeno goes on.
        flush();
        $say($answer);
    }
}

zeno_main();
