# A serial port

A serial port is how a board talks: to a terminal on a desk, to a GPS
module, to a modem or to another board.

```zeno
from board import serial

let port = serial(1, 115200 baud)
port.write("ready\n")
var byte = port.read_byte()
var line = ""
while byte is not none and byte != 10 {
    line = line + char(byte)
    byte = port.read_byte()
}
port.write(f"got {line}\n")
```

The reasons for each choice are in chapter 46 of
[`decisions.md`](decisions.md). The example
[`examples/console.zeno`](../examples/console.zeno) is a small command
line over a serial port that drives the LED.

## Opening one

`serial(n, speed)` is the port with that number, at a speed in `baud`.
It gives a `Serial`. The speed must have its unit:

```
error: the speed of a serial port is in baud, as in `serial(1, 115200 baud)`, and this is int
```

A port the board does not have, or a speed of 0 or below, is a
`ValueError`. The simulated board has ports 0 to 3.

## Using one

| Write | Does |
|---|---|
| `port.write("hi\n")` | sends the bytes of the text, and no line end of its own |
| `port.write_bytes(frame)` | sends a `list[u8]` as it is |
| `port.read_byte()` | the next byte, waiting for it: a `u8 or none` |
| `port.has_byte()` | has a byte come that has not been read? It does not wait |
| `port.number` | which port it is |

`read_byte` gives `none` only when no more bytes can ever come. On a
board that is never. A program that has other work to do asks
`has_byte()` first:

```zeno
from board import serial, led
from time import sleep

let port = serial(1, 115200 baud)
for turn in range(4) {
    if port.has_byte() {
        let byte = port.read_byte()
        if byte is not none {
            port.write_bytes([byte])
        }
    }
    led.toggle()
    sleep(5 ms)
}
```

`Serial` is a type, so a function can take a port, and ports go in
lists and fields.

## On a computer

Every serial port of the simulated board is the terminal, as a serial
monitor is. What is sent is written out as it is, and what is read is
what is typed, a byte at a time. The end of what was typed (Ctrl-D, or
the end of a file piped in) is the end of the port: `read_byte` gives
`none` and `has_byte` is `false`.

```sh
printf 'on\nstatus\n' | zeno run examples/console.zeno
```

A terminal hands over a line at a time, so on a computer `has_byte`
waits for the next line.

## On a board

A compiled program with a serial port calls four more functions of the
board layer ([`board.md`](board.md)), and needs `ZENO_SERIALS`, the
number of ports:

```c
#define ZENO_SERIALS 2
static void zeno_board_serial_open(int port, double baud);
static void zeno_board_serial_write(int port, const uint8_t *bytes, size_t count);
static bool zeno_board_serial_ready(int port);   /* has a byte come? no waiting */
static int zeno_board_serial_read(int port);     /* the next byte, waiting; -1 for no more */
```

With ST's HAL for an STM32 they could be, for one port (a sketch, not
tried on hardware):

```c
extern UART_HandleTypeDef huart2;

static void zeno_board_serial_open(int port, double baud) {
    (void)port;
    huart2.Init.BaudRate = (uint32_t)baud;
    HAL_UART_Init(&huart2);
}
static void zeno_board_serial_write(int port, const uint8_t *bytes, size_t count) {
    (void)port;
    HAL_UART_Transmit(&huart2, (uint8_t *)bytes, (uint16_t)count, HAL_MAX_DELAY);
}
static bool zeno_board_serial_ready(int port) {
    (void)port;
    return __HAL_UART_GET_FLAG(&huart2, UART_FLAG_RXNE);
}
static int zeno_board_serial_read(int port) {
    (void)port;
    uint8_t byte;
    HAL_UART_Receive(&huart2, &byte, 1, HAL_MAX_DELAY);
    return byte;
}
```

Only a program with a port calls them, so a header written without
them still builds every program that has none. The test
`a_serial_port_on_a_board_of_its_own` builds a program over a header
whose port is wired back to itself: what is sent comes back to be read.

## In a built program

A port is its number, an `int64_t`. `port.write("hi")` is one call,
`zeno_serial_write(1, "hi")`, over `zeno_board_serial_write`. Every
engine and every C and LLVM build print the same.

## Not yet

`read_line`, a timeout, reading many bytes at once, `1 / 9600 baud` as
the time of a bit, parity and stop bits, and a byte arriving by
interrupt. See 46.7 in [`decisions.md`](decisions.md).
