/* The checksum of Modbus RTU, as a board's firmware already has it.
   examples/modbus.zeno uses it through `import c` (docs/c.md). */
#include <stddef.h>
#include <stdint.h>

uint16_t modbus_crc(const uint8_t *frame, size_t length) {
    uint16_t crc = 0xFFFF;
    for (size_t i = 0; i < length; i++) {
        crc ^= frame[i];
        for (int bit = 0; bit < 8; bit++) {
            crc = (crc & 1) ? (uint16_t)((crc >> 1) ^ 0xA001) : (uint16_t)(crc >> 1);
        }
    }
    return crc;
}

/* A frame is good when its last two bytes are the checksum of the rest,
   low byte first. */
int modbus_good(const uint8_t *frame, size_t length) {
    if (length < 4) return 0;
    uint16_t crc = modbus_crc(frame, length - 2);
    return frame[length - 2] == (crc & 0xFF) && frame[length - 1] == (crc >> 8);
}
