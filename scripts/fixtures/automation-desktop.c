/* Isolated X11 desktop fixture. Compiled only by test-desktop-automation.mjs. */
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>
#include <stdio.h>
#include <string.h>

int main(void) {
  Display *display = XOpenDisplay(NULL);
  if (!display) return 2;
  int screen = DefaultScreen(display);
  Window window = XCreateSimpleWindow(display, RootWindow(display, screen),
    0, 0, 800, 600, 0, 0, 0x285478);
  XStoreName(display, window, "LocalLM isolated desktop fixture");
  XSelectInput(display, window, ExposureMask | KeyPressMask | ButtonPressMask);
  XMapWindow(display, window);
  XSync(display, False);
  XSetInputFocus(display, window, RevertToParent, CurrentTime);
  GC gc = XCreateGC(display, window, 0, NULL);
  XSetForeground(display, gc, WhitePixel(display, screen));
  char typed[256] = "";
  setvbuf(stdout, NULL, _IOLBF, 0);
  puts("ready");
  for (;;) {
    XEvent event;
    XNextEvent(display, &event);
    if (event.type == KeyPress) {
      KeySym key; char text[32];
      int count = XLookupString(&event.xkey, text, sizeof(text), &key, NULL);
      printf("key %lu %u\n", key, event.xkey.state);
      if (!(event.xkey.state & ControlMask)) {
        for (int i = 0; i < count && strlen(typed) < sizeof(typed)-1; i++) {
          if (text[i] >= 32 && text[i] <= 126) {
            size_t length = strlen(typed); typed[length] = text[i]; typed[length+1] = 0;
          }
        }
      }
    }
    if (event.type == ButtonPress) printf("button %u %d %d\n", event.xbutton.button, event.xbutton.x, event.xbutton.y);
    XClearWindow(display, window);
    XDrawString(display, window, gc, 30, 40, "LocalLM desktop automation test", 30);
    XDrawString(display, window, gc, 30, 80, typed, strlen(typed));
    XFlush(display);
  }
}
