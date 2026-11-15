# How the board and the clock work

`examples/blink.zeno` was written on the first day, before there was a
lexer. It is the program Zeno is for:

```
from board import led
from time import sleep

while true {
    led.toggle()
    sleep(500 ms)
}
```

It runs now.

```sh
cargo run -- run examples/blink.zeno                           # stop it with Ctrl-C
cargo run -- build --firmware --no-collector examples/blink.zeno   # blink.c, and ./blink
```

```
led on
led off
led on
...
```

A computer has no LED on pin 13, so the board is a **simulated** one.
It prints a line each time a pin changes level, which is what a logic
analyser clipped to the pin would show. The same program, compiled,
can have a real board put underneath it without changing a line of
Zeno. This page is about how both of those work.

## Two modules

| | | |
|---|---|---|
| `board` | `led` | the pin the LED is on |
| | `pin(7)` | the pin with that number. A `ValueError` if the board has none |
| | `register(u32, 0x4002_0014)` | the register at that address, read and written inside `unsafe { }` ([`unsafe.md`](unsafe.md)) |
| `time` | `sleep(500 ms)` | wait. The time must have a unit |
| | `now()` | the reading of a clock that only goes forward, in `ms` |

And what a pin can do:

| | |
|---|---|
| `p.on()`, `p.off()` | drive it high or low |
| `p.toggle()` | the other level from the one it has |
| `p.is_on()` | is it high? A `bool` |
| `p.number` | which pin it is. An `int` |

`Pin` is a type, so a function can ask for one:

```
function flash(light: Pin, times: int) {
    for i in range(times) {
        light.toggle()
        sleep(100 ms)
    }
}
```

`sleep(500)` does not compile, and neither does `sleep(3 V)`. Nobody
can tell whether 500 is seconds or milliseconds, and firmware has been
lost to exactly that. The unit is in the type, so the mistake is found
by the checker and not on the bench.

`now()` counts from some fixed moment: on a microcontroller, from when
it was switched on. Only the difference between two readings means
anything:

```
let start = now()
do_the_work()
if now() - start > 20 ms { print("too slow") }
```

## A pin is a number

The first question was what a pin *is* inside the compiler. There were
three candidates:

1. **An object of a class `Pin`**, with its level in a field. Every
   stage already knows objects. But an object lives in memory from the
   collector, and `blink` would then need a collector to switch an LED.
2. **A plain `int`**, with `on(13)` and `off(13)` as functions. Free at
   run time, but then `on(count)` compiles, and so does `led + 1`.
3. **A kind of its own, which is a number underneath.** The checker
   keeps pins and ints apart. After the checker, a pin is its number,
   and costs what an `int` costs.

Zeno takes the third (decision 3.15). It is what units already do:
`500 ms` is a `double` by the time it is compiled, and the checker has
long since made sure nobody added it to a voltage.

So `Pin` went through the stages like this:

| Stage | What was added |
|---|---|
| Checker | `Ty::Pin`, its four methods and `.number`, and `Pin` as a type name |
| Interpreter and VM | `Value::Pin(i64)`. Both use `builtins.rs`, so the methods are written once |
| C backend | `Kind::Pin` is an `int64_t`. `led.on()` is `zeno_pin_on(13)` |
| LLVM backend | `Kind::Pin` is an `i64`. `led.on()` is `call void @zeno_pin_on(i64 13)` |

In both backends nearly every place that handles a kind needed one
word added: where it says `Kind::Int`, it now says
`Kind::Int | Kind::Pin`. The one place a pin differs from an int is how
it is shown: `print(led)` is `Pin(13)`.

That is why a list of pins, a pin in a class, and `Pin or none` all
work without anything written for them. They work for ints.

## The simulated board

`src/board.rs`, sixty lines. It is the whole board for `zeno run`:

```rust
pub struct Board {
    levels: [bool; PINS as usize],
}

pub fn write(&mut self, pin: i64, level: bool) -> Option<String> {
    if self.levels[pin as usize] == level {
        return None;
    }
    self.levels[pin as usize] = level;
    Some(format!("{} {}", name(pin), if level { "on" } else { "off" }))
}
```

Sixty-four pins, all low to begin with, and the LED on pin 13 (where
an Arduino Uno has it, which makes it the number most people have met).

A line is printed only when a level **changes**. `led.on()` twice
prints `led on` once. A pin that is already high does not move, and an
analyser would show nothing.

The interpreter and the VM each hold a `Board`, and reach it through
the `Machine` trait they already share, which gained one function:

```rust
fn board(&mut self) -> &mut crate::board::Board;
```

A board that belonged to the whole process would be simpler still. But
the tests run every program on the interpreter and then on the VM, and
the second run must start with every pin low again.

## Four functions

A compiled program meets the board at the end of `runtime.c`, and this
is the part worth reading slowly:

```c
/*   zeno_board_write(pin, level)   drive a pin high or low
 *   zeno_board_read(pin)           is the pin high?
 *   zeno_board_sleep(seconds)      wait
 *   zeno_board_now()               a clock that only goes forward, in ms
 */
```

Those four, and `ZENO_PINS`, the number of pins, are everything Zeno
knows about hardware. What the program calls is written on top of them
once, for every board:

```c
static void zeno_pin_on(int64_t pin) { zeno_board_write((int)pin, true); }
static void zeno_pin_off(int64_t pin) { zeno_board_write((int)pin, false); }
static void zeno_pin_toggle(int64_t pin) { zeno_board_write((int)pin, !zeno_board_read((int)pin)); }
static bool zeno_pin_is_on(int64_t pin) { return zeno_board_read((int)pin); }
```

`pin(n)` checks `n` against `ZENO_PINS` and raises the `ValueError`.
`zeno_sleep` throws away a time that is zero or negative. So a board
never sees a pin it does not have or a wait that makes no sense, and
the four functions can be four lines each.

The simulated board is the same board as the Rust one, in C, and it is
there unless something else is asked for:

```c
#ifdef ZENO_BOARD
#include ZENO_BOARD
#else
/* ... the simulated board ... */
#endif
```

Before it sleeps, the simulated board flushes what has been printed.
Without that, `./blink | cat` shows nothing for a long time and then
everything at once, because C saves up output that is not going to a
terminal.

## What `blink` compiles to

All of `zeno_program` in `blink.c`:

```c
while (true) {
    zeno_pin_toggle(ZENO_LED);
    zeno_sleep((500.0 / 1000.0));
}
```

And in `blink.ll`, from `zeno build --llvm`:

```llvm
do1:
  call void @zeno_pin_toggle(i64 13)
  %t1 = sitofp i64 500 to double
  %t2 = fdiv double %t1, 1000.0
  call void @zeno_sleep(double %t2)
  br label %while1
```

The board counts in seconds whatever unit the program wrote, so the
compiler turns `500 ms` into seconds on the way, and the C compiler
folds the division to `0.5`.

Nothing in it allocates, so `--no-collector` accepts it, and nothing
in it can raise, so `--firmware` has nothing to add. A pin being a
number is what makes this true.

## Putting a real board underneath

Write the four functions for the board in a header, and tell the C
compiler where it is:

```sh
cargo run -- build --firmware --no-collector examples/blink.zeno
cc -O2 -DZENO_BOARD='"my_board.h"' -o blink blink.c -lm
```

The test `a_board_of_its_own_can_be_put_underneath` does exactly this
with a board of two pins that writes down every call, and whose clock
moves only when the program sleeps:

```c
#define ZENO_PINS 2
static bool my_levels[ZENO_PINS];
static double my_clock = 0.0;
static int my_pin(int pin) { return pin == ZENO_LED ? 0 : pin; }
static bool zeno_board_read(int pin) { return my_levels[my_pin(pin)]; }
static void zeno_board_write(int pin, bool level) {
    my_levels[my_pin(pin)] = level;
    printf("write %d %d\n", my_pin(pin), (int)level);
}
static void zeno_board_sleep(double seconds) { my_clock += seconds * 1000.0; }
static double zeno_board_now(void) { return my_clock; }
```

The same flag works for an LLVM build: add it to the `clang` line.

Two things to see in it. To the Zeno program the LED is always pin 13,
and the board is free to wire that number to whichever pin its LED is
really on (`my_pin`). And a clock that only moves when asked makes a
program that waits run in no time, which is how firmware is tested on a
desk.

For a real microcontroller the four functions become calls to the
vendor's library. With ST's HAL for an STM32, where the LED of a Nucleo
board is on port A pin 5, they would look like this. **This is a
sketch. It has not been run on hardware.**

```c
#define ZENO_PINS 16

static void zeno_board_write(int pin, bool level) {
    if (pin == ZENO_LED) pin = 5;
    HAL_GPIO_WritePin(GPIOA, (uint16_t)(1u << pin), level ? GPIO_PIN_SET : GPIO_PIN_RESET);
}
static bool zeno_board_read(int pin) {
    if (pin == ZENO_LED) pin = 5;
    return HAL_GPIO_ReadPin(GPIOA, (uint16_t)(1u << pin)) == GPIO_PIN_SET;
}
static void zeno_board_sleep(double seconds) { HAL_Delay((uint32_t)(seconds * 1000.0)); }
static double zeno_board_now(void) { return (double)HAL_GetTick(); }
```

`zeno build` does not run the cross compiler for you. It writes C, and
building C for a target is a job the target's own tools already do well
(decision 5.12c). The reasons for the four functions are in 5.12.

## The tests

- `pins_and_the_clock_match` runs one program through everything: the
  interpreter, the VM, C, firmware C and LLVM. It puts pins in a list,
  in a class and in a `Pin or none`.
- `a_pin_the_board_does_not_have_is_an_error` checks the `ValueError`,
  caught and uncaught, at the same line and column everywhere.
- `a_board_of_its_own_can_be_put_underneath` is the header above, on
  all three kinds of C build.
- `blink_is_three_lines_of_c_and_needs_no_collector` holds the C for
  `blink` to exactly the three lines shown.
- `examples/signal.zeno`, a traffic light as a state machine, is built
  every way, including with no collector.

No test can run `blink` itself, because it never ends. The test that
runs every example steps over it.

A clock is hard to test: two runs never read the same time. So the
programs in the tests print only what must be true of any clock, such
as `now() - start >= 200 ms` after sleeping that long.

## What it does not do yet

- **Inputs.** `is_on()` reads back what the program wrote. There is no
  button, and no way to say a pin is an input.
- **Everything else on a microcontroller**: interrupts, timers, PWM, an
  ADC, a serial port. Each would be a few more functions in the board
  layer. Until then a program can reach the hardware itself, through
  registers at the addresses the data sheet gives
  ([`unsafe.md`](unsafe.md)).
- **The runtime is still a desktop runtime.** A firmware build with no
  collector has no `setjmp`, no garbage collector and no `malloc`
  ([`pool.md`](pool.md)), but it still calls `printf` and `exit`
  ([`no-collector.md`](no-collector.md)).
  The board layer is the first piece of a port, and not the whole of
  it.
- **`now()` is a `double`** of milliseconds. That is exact for
  thousands of years, and slow on a processor with no floating point.
  Whole numbers with a size, such as `u32`, came in the stage after
  this one ([`sized-ints.md`](sized-ints.md)). The clock has not been
  moved onto them yet.

## Things to try

1. Run `examples/signal.zeno`, then change one `hold` to return
   `40` with no unit. Read the message. Now try `40 V`.
2. `led.on()` twice prints one line. Find the two places that decide
   that, one in Rust and one in C. Change both to print every write.
   Which tests fail, and is the new output more useful or less?
3. Add `button` to the `board` module: a pin whose level the program
   reads but does not drive. On the simulated board, let it be pressed
   every third time it is read. You will need a fifth function in the
   board layer, or will you?
4. Write `my_board.h` for a board with eight pins where every write
   prints all eight levels as one byte in binary, such as `00100000`.
   Build `signal.zeno` on it by hand with `-DZENO_BOARD`.
5. `sleep` takes any unit of time. Find where `500 ms` becomes seconds
   in `emit_c.rs` (`device`). What does `sleep(2 min)` compile to?
   Where is the multiplication done: in Zeno's compiler, in the C
   compiler, or when the program runs?
6. Add `p.pulse(10 ms)`: on, wait, off. It can be written in the
   runtime on top of the four functions, with nothing new in the board
   layer. Do it in the interpreter first, then C, then LLVM. The test
   `pins_and_the_clock_match` will hold all five to the same answer.
7. `now() - start` is a `float` subtraction. On a small processor
   `HAL_GetTick()` is a `uint32_t` that wraps round after 49 days. What
   does `now() - start` give across the wrap if `zeno_board_now` just
   returns it as a `double`? How would you fix it inside the board
   layer, so that Zeno programs never see it?
8. A big one: an interrupt. `on_rising(pin(2), function() { ... })`
   would run a Zeno function when a pin goes high. What may that
   function not do? (Think about the collector, and about `print`.)
   How could the checker enforce it?
