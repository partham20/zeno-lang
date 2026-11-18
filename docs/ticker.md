# A ticker: a steady beat

A loop that ends with `sleep(10 ms)` takes 10 ms and the time its work
took. A ticker's ticks are 10 ms apart whatever the work takes, so a
loop that waits for them runs at a steady rate, and is told when it
was late.

```zeno
from board import ticker

let beat = ticker(10 ms)
for round in range(5) {
    let came = beat.wait()
    if came > 1 {
        print("late by", came - 1)
    }
}
```

The reasons for each choice are in chapter 49 of
[`decisions.md`](decisions.md). The example
[`examples/steady.zeno`](../examples/steady.zeno) is a loop whose work
runs over once, and notices.

## What a ticker has

| Write | Gives |
|---|---|
| `ticker(10 ms)` | a beat with that long from one tick to the next: a `Ticker`. Its first tick is one period from now |
| `beat.wait()` | waits for the next tick, then gives how many ticks have come since the program last took them: `1` when it kept up, more when it was late |
| `beat.ticks()` | the same count without waiting: `0` when no tick has come |
| `beat.restart()` | the next tick is one period from now, and the ticks that had come are forgotten |
| `beat.period` | the time from one tick to the next, in `ms` |

The period has a unit, and a bare number is refused:

```
error: the period of a ticker has a unit, as in `ticker(10 ms)`, and this is int
```

The ticks stay where they are due. A program that takes one late does
not move the next, so nothing drifts, and the ticks it missed are
counted, never run one after another in a burst.

## Three ways to use one

**A loop at a steady rate** waits:

```zeno
from board import ticker

let beat = ticker(20 ms)
var late = 0
for round in range(3) {
    if beat.wait() > 1 {
        late += 1
    }
}
print(late >= 0)
```

**A loop with several beats** never waits, and asks each:

```zeno
from board import led, ticker
from time import sleep

let blink = ticker(20 ms)
let sample = ticker(5 ms)
var samples = 0
var blinks = 0
while blinks < 2 {
    if blink.ticks() > 0 {
        led.toggle()
        blinks += 1
    }
    samples += sample.ticks()
    sleep(1 ms)
}
print(samples >= 7)
```

`samples += sample.ticks()` loses no tick, however long a round took.

**A timeout** restarts the ticker each time something happens. When a
tick has come, nothing happened for a whole period:

```zeno
from board import input_pin, pull_up, ticker
from time import sleep

let button = input_pin(2, pull_up)
let quiet = ticker(30 ms)
while quiet.ticks() == 0 {
    if button.is_low() {
        quiet.restart()
    }
    sleep(1 ms)
}
print("nothing pressed for", quiet.period)
```

`Ticker` is a type, so a function can ask for one.

## What is refused while the program runs

Each is a `ValueError`:

| The program | The message |
|---|---|
| `ticker(0 ms)` | `the period of a ticker must be above 0 s` |
| a seventeenth ticker | `this board keeps 16 tickers, and this would be one more. Make each ticker once, outside any loop` |

A ticker is not given back. One made inside a loop, or in a function
called again and again, uses the sixteen up.

## On a computer

The clock is the computer's, and a ticker prints nothing. A program in
a page cannot `wait`, as it cannot `sleep`: it asks with `ticks()`.

## On a board

Nothing new is asked of the board. A ticker is kept on the clock and
the sleep a board header already has ([`board.md`](board.md)):

```c
static void zeno_board_sleep(double seconds);
static double zeno_board_now(void);            /* in ms */
```

so it ticks as finely as those two allow. A header may say how many
tickers the program can make:

```c
#define ZENO_TICKERS 4        /* 16 if the header says nothing */
```

Each is two doubles. Only a program with a ticker keeps the table.

## In a built program

A ticker is a number, an `int64_t`: which of the board's tickers it
is. `beat.wait()` is one call, `zeno_ticker_wait(0)`, which sleeps
until the tick and counts what has come.

## Not yet

A function the board runs at each tick (an interrupt), giving a ticker
back, a single tick, and a ticker in the target languages. No ticker
has run on a real board. See 49.7 in [`decisions.md`](decisions.md).
