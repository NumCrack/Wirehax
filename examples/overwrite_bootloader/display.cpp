#include "display.hpp"
#include "font.hpp"

volatile uint16_t* const Display::LCD_CMD  = reinterpret_cast<volatile uint16_t*>(0x60000000);
volatile uint16_t* const Display::LCD_DATA = reinterpret_cast<volatile uint16_t*>(0x60020000);

void Display::init() {
    // The LCD controller is already running and backlit from rescue DFU.
    // Ensure orientation is landscape: MADCTL (0x36) = 0xA0
    writeCommand(0x36);
    writeData(0xA0);

    // Reset address window to full screen
    setWindow(0, 0, WIDTH - 1, HEIGHT - 1);
}

void Display::setWindow(uint16_t x0, uint16_t y0, uint16_t x1, uint16_t y1) {
    // Column address set (CASET)
    writeCommand(0x2A);
    writeData(x0 >> 8);
    writeData(x0 & 0xFF);
    writeData(x1 >> 8);
    writeData(x1 & 0xFF);

    // Row address set (RASET)
    writeCommand(0x2B);
    writeData(y0 >> 8);
    writeData(y0 & 0xFF);
    writeData(y1 >> 8);
    writeData(y1 & 0xFF);

    // Memory write (RAMWR)
    writeCommand(0x2C);
}

void Display::clear(uint16_t color) {
    fillRect(0, 0, WIDTH, HEIGHT, color);
}


void Display::fillRect(int x, int y, int w, int h, uint16_t color) {
    if (x >= WIDTH || y >= HEIGHT || w <= 0 || h <= 0) return;
    if (x < 0) { w += x; x = 0; }
    if (y < 0) { h += y; y = 0; }
    if (x + w > WIDTH)  { w = WIDTH - x; }
    if (y + h > HEIGHT) { h = HEIGHT - y; }
    if (w <= 0 || h <= 0) return;

    setWindow(static_cast<uint16_t>(x), static_cast<uint16_t>(y),
              static_cast<uint16_t>(x + w - 1), static_cast<uint16_t>(y + h - 1));

    uint32_t total = static_cast<uint32_t>(w) * static_cast<uint32_t>(h);
    while (total--) {
        *LCD_DATA = color;
    }
}

void Display::drawRect(int x, int y, int w, int h, uint16_t color) {
    fillRect(x, y, w, 1, color);
    fillRect(x, y + h - 1, w, 1, color);
    fillRect(x, y, 1, h, color);
    fillRect(x + w - 1, y, 1, h, color);
}

void Display::drawChar(int x, int y, char c, uint16_t fg, uint16_t bg) {
    if (x + 8 > WIDTH || y + 8 > HEIGHT || x < 0 || y < 0) {
        return;
    }

    uint8_t uc = static_cast<uint8_t>(c);
    if (uc > 127) uc = '?';

    const uint8_t* glyph = font8x8_basic[uc];

    setWindow(static_cast<uint16_t>(x), static_cast<uint16_t>(y),
              static_cast<uint16_t>(x + 7), static_cast<uint16_t>(y + 7));

    for (int row = 0; row < 8; ++row) {
        uint8_t rowBits = glyph[row];
        for (int col = 0; col < 8; ++col) {
            uint16_t pixelColor = (rowBits & (1 << col)) ? fg : bg;
            *LCD_DATA = pixelColor;
        }
    }
}

void Display::drawString(int x, int y, const char* str, uint16_t fg, uint16_t bg) {
    int curX = x;
    int curY = y;
    while (*str) {
        if (*str == '\n') {
            curX = x;
            curY += 10;
        } else {
            drawChar(curX, curY, *str, fg, bg);
            curX += 8;
        }
        str++;
    }
}
