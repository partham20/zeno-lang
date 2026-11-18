# Inputs: a pin the program reads

A pin could be driven since the first day: `led.on()`, `pin(7).off()`.
Now a pin can be read, so a button, a switch, a door contact or any
sensor that says yes or no can reach a program:

```zeno
from board import input_pin, pull_up, led

let button = input_pin(2, pull_up)    # pressed joins pin 2 to ground
if button.is_low() {
    led.on()
}
print(button, button.is_high())       # InputPin(2) true
```

The reasons for each choice are in chapter 45 of
[`decisions.md`](decisions.md). The example
[`examples/doorbell.zeno`](../examples/doorbell.zeno) counts presses of
a button and rings a bell for each.

## Making one

| Write | Gives |
|---|---|
| `input_pin(2, pull_up)` | pin 2 as an input, held high by a pull-up |
| `input_pin(2, pull_down)` | pin 2 as an input, held low by a pull-down |
| `input_pin(2)` | pin 2 as an input that nothing pulls: the circuit holds its level |

A pin the board does not have is a `ValueError`, as for `pin( )`.

`pull_up` and `pull_down` come from the `board` module, as `led` does,
and are of the type `Pull`, so a function can take one:

```zeno
from board import input_pin, pull_up

function switch(number: int, how: Pull) -> InputPin {
    return input_pin(number, how)
}
let door = switch(4, pull_up)
```

## Reading one

| Write | Gives |
|---|---|
| `b.is_high()` | `true` when the pin is high now |
| `b.is_low()` | `true` when the pin is low now |
| `b.number` | which pin it is, an `int` |

Each read asks the board again, so a loop that reads a button sees it
change. A button that joins its pin to ground reads low while it is
pressed. `InputPin` is a type, so a function can ask for one, and input
pins go in lists, fields and `InputPin or none`.

## An input is not an output

```
error: an InputPin is read and not driven, so it has no `on`. Read it with `is_high()` or `is_low()`. A pin the program drives is made with `pin( )`
```

```
error: `is_low` reads an input, and this Pin is driven by the program. Ask it what it was driven to with `is_on()`. A pin the program reads is made with `input_pin( )`
```

## On a computer

`zeno run` has a simulated board, and nothing presses its buttons. A
pull-up reads high, a pull-down low, and a pin nothing pulls keeps the
level it had. Making an input prints nothing: the lines the board
prints are what the program drives, and a pull is not driven.

## On a board

A compiled program that makes an input pin calls a fifth function of
the board layer ([`board.md`](board.md)), beside the four it had:

```c
static void zeno_board_input(int pin, int pull);   /* 0 nothing, 1 up, 2 down */
```

and reads the pin with `zeno_board_read`. With ST's HAL for an STM32
it could be (a sketch, not tried on hardware):

```c
static void zeno_board_input(int pin, int pull) {
    GPIO_InitTypeDef init = {0};
    init.Pin = (uint16_t)(1u << pin);
    init.Mode = GPIO_MODE_INPUT;
    init.Pull = pull == 1 ? GPIO_PULLUP : pull == 2 ? GPIO_PULLDOWN : GPIO_NOPULL;
    HAL_GPIO_Init(GPIOA, &init);
}
```

Only a program that makes an input pin calls it, so a header written
with four functions still builds every program that reads no pin.

A board header is also how a button is pressed on a desk. The test
`a_button_is_pressed_on_a_board_of_its_own` builds the doorbell over a
header whose button is down for three reads and whose clock moves only
when the program sleeps; the bell rings once, at 10 ms.

## In a built program

An input pin is its number, an `int64_t`, as a pin is. `is_low()` is
one call, `zeno_input_is_low(2)`, over `zeno_board_read`. Every engine
and every C and LLVM build print the same.

## Not yet

Pressing a button under `zeno run`, debouncing in the library, and a
`button` value on the board as `led` is. (Waiting for an edge, and
counting edges, are in [`interrupts.md`](interrupts.md).) See 45.7 in [`decisions.md`](decisions.md).
