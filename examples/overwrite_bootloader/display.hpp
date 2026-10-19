#pragma once

#include <stdint.h>
#include <stddef.h>

namespace Color {
    // Convert 8-bit RGB to 16-bit RGB565
    constexpr uint16_t rgb(uint8_t r, uint8_t g, uint8_t b) {
        return ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3);
    }

    constexpr uint16_t Black         = 0x0000;
    constexpr uint16_t White         = 0xFFFF;
    constexpr uint16_t Red           = rgb(255, 0, 0);
    constexpr uint16_t Green         = rgb(0, 220, 50);
    constexpr uint16_t Blue          = rgb(30, 144, 255);
    constexpr uint16_t Yellow        = rgb(255, 220, 0);
    constexpr uint16_t Cyan          = rgb(0, 255, 255);
    constexpr uint16_t Magenta       = rgb(255, 0, 255);
    constexpr uint16_t Orange        = rgb(255, 128, 0);
    constexpr uint16_t NumWorksYellow = rgb(255, 184, 0);
    constexpr uint16_t Background    = rgb(24, 28, 38);
    constexpr uint16_t Surface       = rgb(36, 42, 56);
    constexpr uint16_t SurfaceBorder = rgb(58, 66, 86);
    constexpr uint16_t TextLight     = rgb(230, 235, 245);
    constexpr uint16_t TextDim       = rgb(140, 150, 170);
}

class Display {
public:
    static constexpr int WIDTH  = 320;
    static constexpr int HEIGHT = 240;

    // STM32 Flexible Memory Controller (FMC) Bank 1 LCD interface
    // Command register (RS = 0) at FMC address 0x60000000
    // Data register    (RS = 1) at FMC address 0x60020000 (A16 = 1)
    static volatile uint16_t* const LCD_CMD;
    static volatile uint16_t* const LCD_DATA;

    Display() = default;

    void init();
    void setWindow(uint16_t x0, uint16_t y0, uint16_t x1, uint16_t y1);
    void clear(uint16_t color = Color::Black);
    void fillRect(int x, int y, int w, int h, uint16_t color);
    void drawRect(int x, int y, int w, int h, uint16_t color);
    void drawChar(int x, int y, char c, uint16_t fg, uint16_t bg = Color::Black);
    void drawString(int x, int y, const char* str, uint16_t fg, uint16_t bg = Color::Black);

private:
    static inline void writeCommand(uint8_t cmd) {
        *LCD_CMD = cmd;
    }

    static inline void writeData(uint16_t data) {
        *LCD_DATA = data;
    }
};
