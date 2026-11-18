# A PWM signal: a pin on for a share of the time

A pin is only ever high or low. Switched on and off fast by a timer of
the chip, it dims a lamp, sets the speed of a motor and tells a servo
where to turn. That signal is called PWM (pulse-width modulation), and
the program only says how much of the time the pin is on.

```zeno
from board import pwm_pin

let lamp = pwm_pin(9, 1 kHz)
lamp.duty(0.25)               # on for a quarter of the time

let servo = pwm_pin(10, 50 Hz)
servo.pulse(1500 us)          # on for 1.5 ms of each 20 ms
```

The reasons for each choice are in chapter 48 of
[`decisions.md`](decisions.md). The example
[`examples/dimmer.zeno`](../examples/dimmer.zeno) fades a lamp up and
down and sweeps a servo.

## What a PWM pin has

| Write | Does |
|---|---|
| `pwm_pin(9, 1 kHz)` | pin 9, as a pin that makes a signal 1000 times a second: a `PwmPin`. It is off at first |
| `lamp.duty(0.25)` | on for that share of the time, a `float` from `0.0` (off) to `1.0` (on all the time) |
| `servo.pulse(1500 us)` | on for that long in each period, a time with its unit |
| `lamp.number` | which pin it is |

The frequency has a unit, and a bare number is refused:

```
error: the frequency of a PWM pin has a unit, as in `pwm_pin(9, 1 kHz)`, and this is int
```

A frequency worked out from a period is one too
([`units-made.md`](units-made.md)):

```zeno
from board import pwm_pin

let period = 20 ms
let servo = pwm_pin(10, 1 / period)      # 50 Hz
servo.pulse(2 ms)
```

`duty` and `pulse` are two ways to say one thing. A lamp or a motor is
given a share, and a servo is given a time: 1 ms is one end of its
turn, 1.5 ms the middle and 2 ms the other end, whatever the period.

`PwmPin` is a type, so a function can ask for one:

```zeno
from board import pwm_pin

function half(light: PwmPin) {
    light.duty(0.5)
}

half(pwm_pin(9, 1 kHz))
```

It is not a `Pin`: it has no `on` and no `off`. All the time on is
`duty(1.0)`.

## What is refused while the program runs

Each is a `ValueError`:

| The program | The message |
|---|---|
| `lamp.duty(1.5)` | `a duty is from 0.0 to 1.0, and this is 1.5. For a quarter of the time on, write 0.25` |
| `servo.pulse(30 ms)` at 50 Hz | `a pulse lasts from no time to one whole period, which is 0.02 s at this frequency, and this is 0.03 s` |
| `pwm_pin(9, 0 Hz)` | `the frequency of a PWM signal must be above 0 Hz` |
| a pin no timer reaches | `pin 7 of this board cannot make a PWM signal` |

A share above 1.0 is never quietly taken as 1.0: a wrong sum should
not run a motor at full power.

## On a computer

No timer makes the signal on the simulated board. Each change of it is
printed, as turning a pin on is:

```
pin 9 pwm 0.25 at 1000 Hz
pin 10 pwm 0.075 at 50 Hz
```

Setting the duty a pin already has prints nothing, and so making a
pin that has no signal yet, which starts it off, prints nothing. Every pin can make a signal.

## On a board

The board header has one function ([`board.md`](board.md)):

```c
/* Start or change the signal on a pin. False if this pin cannot make one. */
static bool zeno_board_pwm(int pin, double hertz, double duty);
```

It is called when the pin is made, with a duty of 0, and at each
`duty` and `pulse`, with the same frequency and the new share. The
runtime has checked the ranges, so the board sees a frequency above 0
and a duty from 0 to 1. With ST's HAL for an STM32 it could be (a
sketch, not tried on hardware):

```c
static bool zeno_board_pwm(int pin, double hertz, double duty) {
    if (pin != 9) return false;                        /* only the pin TIM3 channel 1 drives */
    uint32_t period = (uint32_t)(1000000.0 / hertz);   /* the timer counts microseconds */
    __HAL_TIM_SET_AUTORELOAD(&htim3, period - 1);
    __HAL_TIM_SET_COMPARE(&htim3, TIM_CHANNEL_1, (uint32_t)(duty * period));
    HAL_TIM_PWM_Start(&htim3, TIM_CHANNEL_1);
    return true;
}
```

Only a program that makes a PWM pin calls it, so a header without it
still builds every program that makes none.

## In a built program

A PWM pin is its number, an `int64_t`. `lamp.duty(0.25)` is one call,
`zeno_pwm_duty(9, 0.25, ...)`, which checks the range and calls the
board. The runtime keeps the frequency of each pin, eight bytes a pin.

## Not yet

Reading back the duty or the frequency, changing the frequency of a
pin that is made, the timer's resolution, complementary outputs for a
motor bridge, and a trial on a real board. See 48.6 in
[`decisions.md`](decisions.md).
