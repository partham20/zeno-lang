# Reading a voltage: an analog pin

Most of what a board measures arrives as a voltage on a pin: a
potentiometer, a thermistor, a battery through two resistors, a
current through a shunt. The board's converter (its ADC) turns that
voltage into a number, and Zeno hands it over with its unit.

```zeno
from board import analog_pin

let sensor = analog_pin(0)
let level = sensor.voltage()
if level < 1200 mV {
    print("low:", level)
}
print(sensor.count(), "of", sensor.max_count)
```

The reasons for each choice are in chapter 47 of
[`decisions.md`](decisions.md). The example
[`examples/battery.zeno`](../examples/battery.zeno) watches a battery
and lights the LED when it is low.

## What an analog pin has

| Write | Gives |
|---|---|
| `analog_pin(0)` | pin 0, as a pin whose voltage is read: an `AnalogPin` |
| `sensor.voltage()` | what the pin is at now, a quantity in `V` |
| `sensor.count()` | the same as the converter counted it, an `int` |
| `sensor.max_count` | the count that stands for the whole reference voltage |
| `sensor.number` | which pin it is |

Each `voltage()` and each `count()` is a new reading.

The voltage has its unit, so it works with every other quantity
([`units-made.md`](units-made.md)):

```zeno
from board import analog_pin

let shunt = analog_pin(1)
let drop = shunt.voltage()
let current = drop / 100 ohm
print(current.mA, drop * current)
```

and a bare number beside it is refused:

```
error: `<` does not work between a quantity in V and int
```

A share of the scale, as for a potentiometer, comes from the count:

```zeno
from board import analog_pin

let knob = analog_pin(2)
let turned = knob.count() / knob.max_count      # 0.0 to 1.0
print(turned)
```

`AnalogPin` is a type, so a function can ask for one. It is not a
`Pin` or an `InputPin`: it has no `on` and no `is_high`.

## On a computer

Nothing is wired to the simulated board. A pin that is high reads the
whole reference, 3.3 V and a count of 4095; one that is low reads 0 V.
Its converter has 12 bits, and every pin can be read.

## On a board

The board header says how its converter counts, and has two functions
([`board.md`](board.md)):

```c
#define ZENO_ANALOG_BITS 12           /* counts 0 to 4095 */
#define ZENO_ANALOG_REFERENCE 3.3     /* the volts 4095 stands for */
static bool zeno_board_analog(int pin);      /* false if this pin cannot be read */
static int zeno_board_analog_read(int pin);  /* what the converter counts now */
```

With ST's HAL for an STM32 the second could be (a sketch, not tried on
hardware):

```c
static int zeno_board_analog_read(int pin) {
    ADC_ChannelConfTypeDef channel = {0};
    channel.Channel = my_channel_of(pin);
    channel.Rank = ADC_REGULAR_RANK_1;
    HAL_ADC_ConfigChannel(&hadc1, &channel);
    HAL_ADC_Start(&hadc1);
    HAL_ADC_PollForConversion(&hadc1, HAL_MAX_DELAY);
    return (int)HAL_ADC_GetValue(&hadc1);
}
```

A pin the converter cannot reach is a `ValueError` when the program
makes it: `pin 7 of this board cannot read a voltage`. Only a program
that reads a voltage calls these, so a header without them still
builds every program that reads none.

## In a built program

An analog pin is its number, an `int64_t`. `sensor.voltage()` is one
call, `zeno_analog_voltage(0)`: the count's share of the largest
count, times the reference. Every engine and every C and LLVM build
print the same digits.

## Not yet

Feeding a voltage to a pin under `zeno run`, averaging and sample
times, several pins in one sweep, and writing a voltage out (a DAC,
PWM). See 47.5 in [`decisions.md`](decisions.md).
