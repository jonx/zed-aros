/*
 * gpui_aros_glue.c - flat C wrapper around AROS Intuition/CyberGraphics for
 * the gpui_aros platform backend.
 *
 * Compiled by build.rs (cc-rs) only when targeting AROS. The CFLAGS come from
 * the workspace .cargo/config.toml: host clang with an ELF triple, large code
 * model and x18 reserved, because this object runs inside AROS processes on
 * the hosted (darwin-aarch64) port.
 */

#include <proto/exec.h>
#include <proto/intuition.h>
#include <proto/cybergraphics.h>

#include <intuition/intuition.h>
#include <cybergraphx/cybergraphics.h>
#include <exec/libraries.h>

#include <string.h>
#include <unistd.h>

/* Library bases. The proto headers declare these extern; this module owns
 * them and opens the libraries lazily on first window open. */
struct IntuitionBase *IntuitionBase;
struct Library *CyberGfxBase;

typedef struct GpaWindow {
    struct Window *win;
    char *title; /* AllocVec'd copy; WA_Title does not copy the string */
} GpaWindow;

/* Event kinds, mirrored by GpaEvent in src/window.rs. */
enum {
    GPA_EVENT_CLOSE = 1,
    GPA_EVENT_NEWSIZE = 2,
    GPA_EVENT_REFRESH = 3,
    GPA_EVENT_MOUSEMOVE = 4,
    GPA_EVENT_MOUSEDOWN = 5,
    GPA_EVENT_MOUSEUP = 6,
    GPA_EVENT_RAWKEY = 7,
};

typedef struct GpaEvent {
    int kind;
    int code;
    int qualifier;
    int x;
    int y;
} GpaEvent;

static int gpa_init(void)
{
    if (!IntuitionBase) {
        IntuitionBase =
            (struct IntuitionBase *)OpenLibrary("intuition.library", 0);
    }
    if (!CyberGfxBase) {
        CyberGfxBase = OpenLibrary("cybergraphics.library", 0);
    }
    return (IntuitionBase && CyberGfxBase) ? 0 : -1;
}

static char *gpa_strdup(const char *s)
{
    if (!s)
        return NULL;
    size_t len = strlen(s) + 1;
    char *copy = AllocVec(len, MEMF_ANY);
    if (copy)
        memcpy(copy, s, len);
    return copy;
}

void *gpa_open_window(int x, int y, int inner_w, int inner_h,
                      const char *title)
{
    if (gpa_init() != 0)
        return NULL;

    GpaWindow *w = AllocVec(sizeof(GpaWindow), MEMF_ANY | MEMF_CLEAR);
    if (!w)
        return NULL;

    w->title = gpa_strdup(title);

    w->win = OpenWindowTags(NULL,
        WA_Left, x,
        WA_Top, y,
        WA_InnerWidth, inner_w,
        WA_InnerHeight, inner_h,
        WA_Title, (IPTR)(w->title ? w->title : ""),
        WA_Flags, WFLG_DRAGBAR | WFLG_DEPTHGADGET | WFLG_CLOSEGADGET
            | WFLG_SIZEGADGET | WFLG_ACTIVATE | WFLG_SMART_REFRESH,
        WA_IDCMP, IDCMP_CLOSEWINDOW | IDCMP_NEWSIZE | IDCMP_REFRESHWINDOW
            | IDCMP_MOUSEBUTTONS | IDCMP_MOUSEMOVE | IDCMP_RAWKEY,
        WA_ReportMouse, TRUE,
        TAG_DONE);

    if (!w->win) {
        FreeVec(w->title);
        FreeVec(w);
        return NULL;
    }
    return w;
}

void gpa_close_window(void *handle)
{
    GpaWindow *w = handle;
    if (!w)
        return;
    if (w->win)
        CloseWindow(w->win);
    FreeVec(w->title);
    FreeVec(w);
}

/* Blit a tightly rowed RGBA8888 buffer into the window's inner area at
 * (x, y). RECTFMT_RGBA matches tiny-skia's RGBA byte order directly. */
void gpa_blit(void *handle, const void *rgba, int src_stride_bytes, int x,
              int y, int width, int height)
{
    GpaWindow *w = handle;
    if (!w || !w->win || width <= 0 || height <= 0)
        return;

    WritePixelArray((APTR)rgba, 0, 0, src_stride_bytes, w->win->RPort,
                    w->win->BorderLeft + x, w->win->BorderTop + y, width,
                    height, RECTFMT_RGBA);
}

/* Nonblocking: translate at most one queued IntuiMessage. Returns 1 when an
 * event was written to *out, 0 when the port is empty. */
int gpa_poll_event(void *handle, GpaEvent *out)
{
    GpaWindow *w = handle;
    if (!w || !w->win || !w->win->UserPort || !out)
        return 0;

    struct IntuiMessage *im;
    while ((im = (struct IntuiMessage *)GetMsg(w->win->UserPort))) {
        int kind = 0;
        int code = im->Code;
        int qualifier = im->Qualifier;
        int mx = im->MouseX - w->win->BorderLeft;
        int my = im->MouseY - w->win->BorderTop;

        switch (im->Class) {
        case IDCMP_CLOSEWINDOW:
            kind = GPA_EVENT_CLOSE;
            break;
        case IDCMP_NEWSIZE:
            kind = GPA_EVENT_NEWSIZE;
            break;
        case IDCMP_REFRESHWINDOW:
            /* Intuition requires the Begin/EndRefresh pair even though we
             * repaint the whole inner area on the next frame anyway. */
            BeginRefresh(w->win);
            EndRefresh(w->win, TRUE);
            kind = GPA_EVENT_REFRESH;
            break;
        case IDCMP_MOUSEMOVE:
            kind = GPA_EVENT_MOUSEMOVE;
            break;
        case IDCMP_MOUSEBUTTONS:
            switch (im->Code) {
            case SELECTDOWN:
                kind = GPA_EVENT_MOUSEDOWN;
                code = 0;
                break;
            case SELECTUP:
                kind = GPA_EVENT_MOUSEUP;
                code = 0;
                break;
            case MENUDOWN:
                kind = GPA_EVENT_MOUSEDOWN;
                code = 1;
                break;
            case MENUUP:
                kind = GPA_EVENT_MOUSEUP;
                code = 1;
                break;
            case MIDDLEDOWN:
                kind = GPA_EVENT_MOUSEDOWN;
                code = 2;
                break;
            case MIDDLEUP:
                kind = GPA_EVENT_MOUSEUP;
                code = 2;
                break;
            default:
                break;
            }
            break;
        case IDCMP_RAWKEY:
            kind = GPA_EVENT_RAWKEY;
            break;
        default:
            break;
        }

        ReplyMsg((struct Message *)im);

        if (kind != 0) {
            out->kind = kind;
            out->code = code;
            out->qualifier = qualifier;
            out->x = mx;
            out->y = my;
            return 1;
        }
        /* Unhandled class: keep draining. */
    }
    return 0;
}

unsigned gpa_window_sigmask(void *handle)
{
    GpaWindow *w = handle;
    if (!w || !w->win || !w->win->UserPort)
        return 0;
    return 1u << w->win->UserPort->mp_SigBit;
}

/* v1 wait: poll SetSignal() + usleep() instead of Wait() on
 * mask | timer.device. Simpler and safe (no timer.device unit setup); the
 * cost is up to ~2 ms of extra latency per wakeup, fine for a 30 fps loop.
 * SetSignal(0, mask) reads and clears the signals in one call. */
void gpa_wait_timeout_ms(unsigned mask, int ms)
{
    int waited_ms = 0;
    for (;;) {
        if (mask && (SetSignal(0, mask) & mask))
            return;
        if (waited_ms >= ms)
            return;
        usleep(2000);
        waited_ms += 2;
    }
}

void gpa_inner_size(void *handle, int *out_w, int *out_h)
{
    GpaWindow *w = handle;
    int width = 0;
    int height = 0;
    if (w && w->win) {
        width = w->win->Width - w->win->BorderLeft - w->win->BorderRight;
        height = w->win->Height - w->win->BorderTop - w->win->BorderBottom;
    }
    if (out_w)
        *out_w = width;
    if (out_h)
        *out_h = height;
}

void gpa_set_title(void *handle, const char *title)
{
    GpaWindow *w = handle;
    if (!w || !w->win)
        return;
    char *copy = gpa_strdup(title);
    if (!copy)
        return;
    SetWindowTitles(w->win, copy, (CONST_STRPTR)~0ul);
    FreeVec(w->title);
    w->title = copy;
}

/* Size of the (public) screen the windows open on. Returns 0 on success. */
int gpa_screen_size(int *out_w, int *out_h)
{
    if (gpa_init() != 0)
        return -1;
    struct Screen *screen = LockPubScreen(NULL);
    if (!screen)
        return -1;
    if (out_w)
        *out_w = screen->Width;
    if (out_h)
        *out_h = screen->Height;
    UnlockPubScreen(NULL, screen);
    return 0;
}
