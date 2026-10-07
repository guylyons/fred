// The C interface in src/gui.rs; see there for what each call does.
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct Gui Gui;

typedef struct {
    uint32_t fg, bg, text_start, text_len, flags;
} FredCell;

typedef struct {
    uint16_t cols, rows, cursor_x, cursor_y;
    bool cursor_visible, cursor_bar;
    const FredCell *cells;
    const uint8_t *text;
    size_t text_len;
} FredFrame;

Gui *fred_new(const char *const *argv, size_t argc);
const char *fred_error(void);
void fred_free(Gui *g);
void fred_key(Gui *g, uint32_t code, uint32_t ch, uint32_t mods);
void fred_paste(Gui *g, const char *text);
void fred_mouse(Gui *g, uint32_t kind, uint16_t col, uint16_t row);
void fred_resize(Gui *g, uint16_t cols, uint16_t rows);
void fred_save(Gui *g);
void fred_close(Gui *g);
uint32_t fred_tick(Gui *g);
FredFrame fred_render(Gui *g);
const char *fred_title(Gui *g);
