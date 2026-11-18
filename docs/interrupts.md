# Interrupts: the board counts a pin's edges

A pulse from a wheel sensor is over in a millisecond. A program that
looks at the pin now and then misses most of them, and one that looks
all the time does nothing else. An interrupt has the chip notice each
edge by itself. In Zeno the board counts them, and the program asks
for the count.

```zeno
from board import input_pin, pin, rising

let sensor = input_pin(3)
let turns = sensor.interrupt(rising)

pin(3).on()                # on a computer the program plays the sensor
pin(3).off()
pin(3).on()

print(turns.edges())       # 2
print(turns.edges())       # 0: they were taken
```

The reasons for each choice are in chapter 50 of
[`decisions.md`](decisions.md). The example
[`examples/tachometer.zeno`](../examples/tachometer.zeno) counts the
turns of a wheel in each stretch of time.

## The rule

**The program's own code never runs inside an interrupt.** The board
counts what happened, and the program asks when it chooses to. No
function starts by itself in the middle of another, so no variable is
shared with one, and there is no `volatile`, no critical section and
no race to find.

## What an interrupt has

| Write | Gives |
|---|---|
| `button.interrupt(falling)` | the edges of that input pin, counted from now on: an `Interrupt` |
| `press.wait()` | waits for an edge, then gives how many have come since the program last took them |
| `press.edges()` | the same count without waiting: `0` when none has come |
| `press.number` | which pin it is |

The edge is `rising` (low to high), `falling` (high to low) or
`either`, from the `board` module. They are values of the type
`Edge`, and a str in their place is refused:

```
error: an interrupt counts `rising`, `falling` or `either` edges, from the `board` module, and this is str
```

The pin is still an input. `button.is_low()` reads it as before
([`inputs.md`](inputs.md)). One pin has one interrupt: a second
`interrupt( )` on it starts the count again, with the new edge.

## Three ways to use one

**Nothing else to do** waits. On a board the chip sleeps until the
edge:

```zeno
from board import falling, input_pin, led, pin, pull_up

let button = input_pin(2, pull_up)
let press = button.interrupt(falling)

pin(2).off()               # stands in for a finger, on a computer

press.wait()
led.toggle()
```

**A loop that does other things** asks, and loses nothing however long
a round takes:

```zeno
from board import falling, input_pin, pin
from time import sleep

let press = input_pin(2).interrupt(falling)
pin(2).on()
var presses = 0
for round in range(3) {
    pin(2).toggle()
    presses += press.edges()
    sleep(1 ms)
}
print(presses)
```

**A rate** is a count taken at each tick of a ticker
([`ticker.md`](ticker.md)):

```zeno
from board import input_pin, pin, rising, ticker

let turns = input_pin(3).interrupt(rising)
let second = ticker(10 ms)
pin(3).on()
second.wait()
print(turns.edges(), "turns in this stretch")
```

A button bounces: one press can be several edges within a few
milliseconds. A program that wants presses takes the count, sleeps
`20 ms`, and takes it again to throw the bounces away.

## What is refused while the program runs

Each is a `ValueError`:

| The program | The message |
|---|---|
| a pin with no interrupt | `pin 7 of this board cannot count its edges` |
| `press.wait()` on a computer, with no edge counted | ``the board says no edge can ever come on pin 2, so the program would wait for ever. On a computer nothing is wired to a pin: drive it with `pin(2)` before the wait`` |

## On a computer

Nothing is wired to the simulated board. The edges it counts are the
ones the program drives on that pin itself, with `pin(2).on()` and
`off()`, as if a wire joined the two. Nothing can drive a pin while
the program waits, so a `wait()` with nothing counted would never end,
and is an error instead.

## On a board

The board header has three functions ([`board.md`](board.md)):

```c
/* Count the pin's edges from now on: 1 rising, 2 falling, 3 either.
 * False if this pin has no interrupt. */
static bool zeno_board_interrupt(int pin, int edge);
/* How many were counted since this was last asked. Start again from none. */
static int zeno_board_edges(int pin);
/* The same, after waiting until at least one has come. */
static int zeno_board_edges_wait(int pin);
```

The chip's own handler is where the counting is done, and it is two
lines. For an STM32 with ST's HAL the whole of it could be (a sketch,
not tried on hardware):

```c
static volatile int my_edges[16];

void HAL_GPIO_EXTI_Callback(uint16_t line) {        /* the chip calls this at each edge */
    my_edges[__builtin_ctz(line)]++;
}

static int zeno_board_edges(int pin) {
    __disable_irq();
    int edges = my_edges[pin];
    my_edges[pin] = 0;
    __enable_irq();
    return edges;
}

static int zeno_board_edges_wait(int pin) {
    for (;;) {
        __disable_irq();
        int edges = my_edges[pin];
        my_edges[pin] = 0;
        if (edges == 0) __WFI();                    /* sleep; an edge that is pending wakes it at once */
        __enable_irq();
        if (edges > 0) return edges;
    }
}
```

Looking at the count and going to sleep is one step there, with the
interrupts off, so an edge between the two is not slept through. That
is why the wait belongs to the board. Only a program with an interrupt
calls these three.

## In a built program

An interrupt is the pin's number, an `int64_t`. `press.edges()` is one
call, `zeno_interrupt_edges(2)`.

## Not yet

A function of the program run at each edge, waiting for the first of
several interrupts and tickers, the time an edge came, and debouncing
in the board. No interrupt has run on a real board. See 50.6 in
[`decisions.md`](decisions.md).
