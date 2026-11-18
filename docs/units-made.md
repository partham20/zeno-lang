# Units made of units: `V * A` is `W`

A quantity in Zeno carries its unit: `3.3 V`, `20 mA`, `500 ms`. Two
quantities of the right kinds now make a third, so Ohm's law, power,
energy and periods are written as they are on paper:

```zeno
let supply = 3.3 V
let draw = 20 mA
let power = supply * draw
print(power)               # 0.066 W
print(power.mW)            # 66 mW
print(supply / draw)       # 165 ohm
print(1 / 2 ms)            # 500 Hz
```

The reasons for each choice are in chapter 43 of
[`decisions.md`](decisions.md).

## What makes what

| Write | Gives | For example |
|---|---|---|
| voltage `*` current | power | `5 V * 2 A` is `10 W` |
| current `*` resistance | voltage | `2 mA * 4.7 kohm` is `9.4 V` |
| power `*` time | energy | `5 W * 2 h` is `36000 J` |
| voltage `/` current | resistance | `5 V / 2 mA` is `2500 ohm` |
| voltage `/` resistance | current | `12 V / 4.7 kohm` is about `0.00255 A` |
| power `/` voltage, power `/` current | current, voltage | `100 W / 230 V` is about `0.435 A` |
| energy `/` time, energy `/` power | power, time | `1 kWh / 1 h` is `1000 W` |
| a number `/` time | frequency | `1 / 2 ms` is `500 Hz` |
| a number `/` frequency | time | `1 / 1 kHz` is `0.001 s` |
| time `*` frequency | a plain number | `10 ms * 1 kHz` is `10.0` |

A product works either way round: `20 mA * 3.3 V` is also `0.066 W`.

## The unit of the answer

The answer is in the unit of its kind with no prefix: `W`, `A`, `V`,
`ohm`, `J`, `s` or `Hz`. Change it with a dot, as for any quantity:

```zeno
let energy = 5 W * 2 h
print(energy)              # 36000 J
print(energy.Wh)           # 10 Wh
let r = 3.3 V / 20 mA
print(r.kohm)              # 0.165 kohm
```

A function says which unit it gives, and the checker holds it to it:

```zeno
function watts(v: V, i: mA) -> W {
    return v * i
}
print(watts(5 V, 120 mA))  # 0.6 W
```

Asking for `mW` where the answer is in `W` is an error that says to
write `.mW`.

## What is refused

Any pair not in the table, such as `2 m * 3 m` or `1 / 2 V`, is an
error before the program runs, and the message lists what works. Zeno
does not invent a unit such as `m^2`: every unit has a name on the
list, so a slip in a sum is still caught.

## In a built program

`zeno build` keeps a quantity a `double` and knows its unit while
compiling, so `supply * draw` is each side scaled by a number known
while building, and one multiplication. Every engine and every target language prints
the same digits.

## Not yet

Charge (`mAh`, for battery life), speed (`m / s`), units a program
names itself, and `sum` of quantities. See 43.6 in
[`decisions.md`](decisions.md).
