/* Simulates the host desktop grabbing the keyboard (like GNOME's Alt+Tab switcher): grabs the
   keyboard for N ms on the root window, then lets go. Test helper for sessao/testar.sh. */
#include <X11/Xlib.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(int argc, char **argv) {
    int ms = argc > 1 ? atoi(argv[1]) : 800;
    Display *d = XOpenDisplay(NULL);
    if (!d) return 1;
    int r = XGrabKeyboard(d, DefaultRootWindow(d), False, GrabModeAsync, GrabModeAsync, CurrentTime);
    XSync(d, False);
    printf("grab %d\n", r);
    fflush(stdout);
    usleep(ms * 1000);
    XUngrabKeyboard(d, CurrentTime);
    XSync(d, False);
    XCloseDisplay(d);
    return 0;
}
