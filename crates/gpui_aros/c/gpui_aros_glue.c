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
#include <proto/keymap.h>
#include <proto/gadtools.h>
#include <proto/asl.h>
#include <proto/dos.h>

#include <intuition/intuition.h>
#include <intuition/pointerclass.h>
#include <cybergraphx/cybergraphics.h>
#include <devices/clipboard.h>
#include <devices/inputevent.h>
#include <exec/io.h>
#include <exec/libraries.h>
#include <graphics/gfx.h>
#include <libraries/asl.h>
#include <libraries/gadtools.h>
#include <workbench/startup.h>

#include <string.h>
#include <unistd.h>

/* Library bases. The proto headers declare these extern; this module owns
 * them and opens the libraries lazily on first window open. */
struct IntuitionBase *IntuitionBase;
struct Library *CyberGfxBase;
/* keymap.library drives rawkey -> character translation (MapRawKey). Opened
 * lazily like the others but non-fatal when missing: windows still work,
 * typing just produces no characters (named keys keep working from codes). */
struct Library *KeymapBase;
/* gadtools builds the menu strips; asl serves the file requesters. Both are
 * lazily opened and non-fatal when missing: without gadtools there are no
 * pulldowns, without asl the path prompts return "cancelled". */
struct Library *GadToolsBase;
struct Library *AslBase;
/* DOSBase comes from the C startup the final binary always links. */

/* Queued MENUPICK actions: an Intuition menu drag can select several items
 * in one message (NextSelect chain); poll_event hands them out one by one. */
#define GPA_PICK_QUEUE 32

typedef struct GpaWindow {
    struct Window *win;
    char *title; /* AllocVec'd copy; WA_Title does not copy the string */

    /* Menu strip state (gpa_set_menus). GadTools does not copy label
     * strings, so `menu_strs` keeps the copies alive until FreeMenus. */
    struct Menu *menustrip;
    APTR vi; /* GetVisualInfo of the window's screen */
    char **menu_strs;
    int menu_str_count;
    int picks[GPA_PICK_QUEUE];
    int pick_head, pick_tail;
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
    GPA_EVENT_MENUPICK = 8, /* code = the item id passed to gpa_set_menus */
};

typedef struct GpaEvent {
    int kind;
    int code;
    int qualifier;
    int x;
    int y;
    /* RAWKEY only, else empty. keymap.library translation of the key in the
     * system charset (ISO-8859-1 on stock AROS), NUL-terminated.
     * `chars` uses the message's real qualifiers (what the user typed);
     * `base_chars` uses qualifier 0 (the unmodified key, for the
     * keybinding name — shift-A must still bind as "a"). */
    char chars[8];
    char base_chars[8];
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
    if (!KeymapBase) {
        KeymapBase = OpenLibrary("keymap.library", 0);
    }
    if (!GadToolsBase) {
        GadToolsBase = OpenLibrary("gadtools.library", 0);
    }
    if (!AslBase) {
        AslBase = OpenLibrary("asl.library", 0);
    }
    /* KeymapBase/GadToolsBase/AslBase deliberately not required (see their
     * comments). */
    return (IntuitionBase && CyberGfxBase) ? 0 : -1;
}

/* Translate a rawkey IntuiMessage through the active keymap into `buf`
 * (NUL-terminated; empty on failure or when the code produces nothing, e.g.
 * a dead key waiting for its successor). `qualifier` lets the caller ask for
 * the typed characters (real qualifiers) or the unmodified key (0).
 * The IAddress dead-key context pointer is only valid before ReplyMsg — call
 * this while the message is still ours. */
static void gpa_map_rawkey(struct IntuiMessage *im, int code, int qualifier,
                           char *buf, int buf_len)
{
    buf[0] = 0;
    if (!KeymapBase)
        return;

    struct InputEvent ie;
    memset(&ie, 0, sizeof(ie));
    ie.ie_Class = IECLASS_RAWKEY;
    ie.ie_Code = code;
    ie.ie_Qualifier = qualifier;
    /* Classic Amiga lore says IAddress points at a pointer to the previous
     * key's codes (the dead-key context). On this AROS the injected-input
     * path (cocoametal control FIFO -> keyboard HIDD) delivers RAWKEY
     * messages whose IAddress is NOT a dereferenceable pointer — chasing it
     * crashed in the field (fault addr 0x28). Dead-key composition is not
     * worth a crash: translate without the context. */
    ie.ie_EventAddress = NULL;
    (void)im;

    LONG n = MapRawKey(&ie, (STRPTR)buf, buf_len - 1, NULL);
    if (n > 0)
        buf[n] = 0;
    else
        buf[0] = 0;
}

/* Main-task wakeup: the run loop parks in gpa_wait_timeout_ms; the
 * dispatcher's worker/timer threads (AROS tasks on the hosted port) nudge it
 * with a dedicated exec signal so main-thread work starts within the poll
 * granularity (~2 ms) instead of the frame budget (~33 ms). */
static struct Task *gpa_main_task;
static LONG gpa_wake_sigbit = -1;

/* Called once from the platform's main thread before the run loop starts. */
void gpa_init_main(void)
{
    gpa_main_task = FindTask(NULL);
    if (gpa_wake_sigbit < 0)
        gpa_wake_sigbit = AllocSignal(-1);
}

unsigned gpa_wake_sigmask(void)
{
    return gpa_wake_sigbit >= 0 ? (1u << gpa_wake_sigbit) : 0;
}

/* Callable from any task/thread; no-op before gpa_init_main. */
void gpa_wake_main(void)
{
    if (gpa_main_task && gpa_wake_sigbit >= 0)
        Signal(gpa_main_task, 1u << gpa_wake_sigbit);
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
            | IDCMP_MOUSEBUTTONS | IDCMP_MOUSEMOVE | IDCMP_RAWKEY
            | IDCMP_MENUPICK,
        WA_ReportMouse, TRUE,
        TAG_DONE);

    if (!w->win) {
        FreeVec(w->title);
        FreeVec(w);
        return NULL;
    }
    return w;
}

static void gpa_free_menus(GpaWindow *w)
{
    if (w->menustrip) {
        if (w->win)
            ClearMenuStrip(w->win);
        FreeMenus(w->menustrip);
        w->menustrip = NULL;
    }
    if (w->vi) {
        FreeVisualInfo(w->vi);
        w->vi = NULL;
    }
    if (w->menu_strs) {
        for (int i = 0; i < w->menu_str_count; i++)
            FreeVec(w->menu_strs[i]);
        FreeVec(w->menu_strs);
        w->menu_strs = NULL;
        w->menu_str_count = 0;
    }
    w->pick_head = w->pick_tail = 0;
}

void gpa_close_window(void *handle)
{
    GpaWindow *w = handle;
    if (!w)
        return;
    gpa_free_menus(w);
    if (w->win)
        CloseWindow(w->win);
    FreeVec(w->title);
    FreeVec(w);
}

/* Blit the (x, y, width, height) subrect of a tightly rowed RGBA8888
 * full-framebuffer into the window's inner area at the same (x, y) — the
 * dirty-rect path hands the whole buffer plus the damaged rect.
 * RECTFMT_RGBA matches tiny-skia's RGBA byte order directly. */
void gpa_blit(void *handle, const void *rgba, int src_stride_bytes, int x,
              int y, int width, int height)
{
    GpaWindow *w = handle;
    if (!w || !w->win || width <= 0 || height <= 0)
        return;

    WritePixelArray((APTR)rgba, x, y, src_stride_bytes, w->win->RPort,
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

    /* Deliver queued menu picks (a NextSelect chain from an earlier
     * MENUPICK message) before touching the port. */
    if (w->pick_head != w->pick_tail) {
        memset(out, 0, sizeof(*out));
        out->kind = GPA_EVENT_MENUPICK;
        out->code = w->picks[w->pick_head];
        w->pick_head = (w->pick_head + 1) % GPA_PICK_QUEUE;
        return 1;
    }

    struct IntuiMessage *im;
    while ((im = (struct IntuiMessage *)GetMsg(w->win->UserPort))) {
        int kind = 0;
        int code = im->Code;
        int qualifier = im->Qualifier;
        int mx = im->MouseX - w->win->BorderLeft;
        int my = im->MouseY - w->win->BorderTop;
        out->chars[0] = 0;
        out->base_chars[0] = 0;

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
        case IDCMP_MENUPICK: {
            /* Queue every selection of the drag (NextSelect chain); the
             * first one is returned below, the rest on later polls. */
            UWORD sel = im->Code;
            while (sel != MENUNULL && w->menustrip) {
                struct MenuItem *item = ItemAddress(w->menustrip, sel);
                if (!item)
                    break;
                int next = (w->pick_tail + 1) % GPA_PICK_QUEUE;
                if (next == w->pick_head)
                    break; /* queue full; drop the tail of the chain */
                w->picks[w->pick_tail] =
                    (int)(IPTR)GTMENUITEM_USERDATA(item);
                w->pick_tail = next;
                sel = item->NextSelect;
            }
            if (w->pick_head != w->pick_tail) {
                kind = GPA_EVENT_MENUPICK;
                code = w->picks[w->pick_head];
                w->pick_head = (w->pick_head + 1) % GPA_PICK_QUEUE;
            }
            break;
        }
        case IDCMP_RAWKEY: {

            kind = GPA_EVENT_RAWKEY;
            /* Map while the message (and its IAddress dead-key context) is
             * still valid — ReplyMsg comes right after this switch. Key-ups
             * (IECODE_UP_PREFIX) map the corresponding down code so the
             * release still carries a key name; typed characters are only
             * meaningful on the down stroke. */
            int down_code = im->Code & ~IECODE_UP_PREFIX;
            if (!(im->Code & IECODE_UP_PREFIX)) {
                gpa_map_rawkey(im, down_code, im->Qualifier, out->chars,
                               sizeof(out->chars));
            }
            gpa_map_rawkey(im, down_code, 0, out->base_chars,
                           sizeof(out->base_chars));
            break;
        }
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

/* ---- Clipboard (clipboard.device unit 0, IFF FTXT) ---------------------
 *
 * The Amiga clipboard is an IFF stream on clipboard.device: text lives in a
 * FORM FTXT containing CHRS chunks (system charset — ISO-8859-1 on stock
 * AROS; the Rust side converts to/from UTF-8). Reads and writes are the
 * classic RKM sequences: sequential CMD_READs until io_Actual == 0, and
 * CMD_WRITEs finished by CMD_UPDATE to publish the clip. Main-thread only
 * (called from GPUI's Platform methods); one lazily-created IORequest. */

static struct MsgPort *clip_port;
static struct IOClipReq *clip_req;

static int clip_open(void)
{
    if (clip_req)
        return 0;
    clip_port = CreateMsgPort();
    if (!clip_port)
        return -1;
    clip_req =
        (struct IOClipReq *)CreateIORequest(clip_port, sizeof(struct IOClipReq));
    if (!clip_req) {
        DeleteMsgPort(clip_port);
        clip_port = NULL;
        return -1;
    }
    if (OpenDevice("clipboard.device", PRIMARY_CLIP,
                   (struct IORequest *)clip_req, 0) != 0) {
        DeleteIORequest((struct IORequest *)clip_req);
        clip_req = NULL;
        DeleteMsgPort(clip_port);
        clip_port = NULL;
        return -1;
    }
    return 0;
}

static ULONG clip_io(UWORD cmd, void *data, ULONG len)
{
    clip_req->io_Command = cmd;
    clip_req->io_Data = (STRPTR)data;
    clip_req->io_Length = len;
    DoIO((struct IORequest *)clip_req);
    return clip_req->io_Actual;
}

static void be32(UBYTE *out, ULONG v)
{
    out[0] = (v >> 24) & 0xff;
    out[1] = (v >> 16) & 0xff;
    out[2] = (v >> 8) & 0xff;
    out[3] = v & 0xff;
}

static ULONG rd32(const UBYTE *p)
{
    return ((ULONG)p[0] << 24) | ((ULONG)p[1] << 16) | ((ULONG)p[2] << 8)
        | (ULONG)p[3];
}

/* Write `len` bytes as FORM FTXT / CHRS. Returns 0 on success. */
int gpa_clipboard_write_text(const void *bytes, int len)
{
    if (len < 0 || clip_open() != 0)
        return -1;

    ULONG chrs_len = (ULONG)len;
    ULONG pad = chrs_len & 1;
    /* FORM length counts everything after its own length field. */
    ULONG form_len = 4 + 8 + chrs_len + pad;

    UBYTE header[20];
    memcpy(header, "FORM", 4);
    be32(header + 4, form_len);
    memcpy(header + 8, "FTXT", 4);
    memcpy(header + 12, "CHRS", 4);
    be32(header + 16, chrs_len);

    clip_req->io_Offset = 0;
    clip_req->io_Error = 0;
    clip_req->io_ClipID = 0;
    clip_io(CMD_WRITE, header, sizeof(header));
    if (chrs_len)
        clip_io(CMD_WRITE, (void *)bytes, chrs_len);
    if (pad) {
        UBYTE zero = 0;
        clip_io(CMD_WRITE, &zero, 1);
    }
    /* Publish. */
    clip_io(CMD_UPDATE, NULL, 0);
    return clip_req->io_Error ? -1 : 0;
}

/* Read the first CHRS chunk of a FORM FTXT clip. On success returns 0 and
 * hands out an AllocVec'd, NUL-terminated buffer (free with gpa_free). */
int gpa_clipboard_read_text(void **out, int *out_len)
{
    *out = NULL;
    *out_len = 0;
    if (clip_open() != 0)
        return -1;

    clip_req->io_Offset = 0;
    clip_req->io_Error = 0;
    clip_req->io_ClipID = 0;

    UBYTE hdr[12];
    if (clip_io(CMD_READ, hdr, sizeof(hdr)) == sizeof(hdr)
        && memcmp(hdr, "FORM", 4) == 0 && memcmp(hdr + 8, "FTXT", 4) == 0) {
        for (;;) {
            UBYTE ch[8];
            if (clip_io(CMD_READ, ch, sizeof(ch)) != sizeof(ch))
                break;
            ULONG clen = rd32(ch + 4);
            if (memcmp(ch, "CHRS", 4) == 0) {
                UBYTE *buf = AllocVec(clen + 1, MEMF_ANY);
                if (buf && clip_io(CMD_READ, buf, clen) == clen) {
                    buf[clen] = 0;
                    *out = buf;
                    *out_len = (int)clen;
                } else {
                    FreeVec(buf); /* FreeVec(NULL) is a no-op */
                }
                break;
            }
            /* Skip a foreign chunk (+ IFF pad byte). */
            ULONG skip = clen + (clen & 1);
            UBYTE sink[256];
            while (skip) {
                ULONG n = skip > sizeof(sink) ? sizeof(sink) : skip;
                if (clip_io(CMD_READ, sink, n) != n)
                    goto drain;
                skip -= n;
            }
        }
    }

drain:
    /* RKM convention: keep reading until io_Actual == 0 so the device knows
     * this read sequence is over (otherwise the clip stays locked). */
    {
        UBYTE sink[256];
        while (clip_io(CMD_READ, sink, sizeof(sink)) > 0) {}
    }
    return *out ? 0 : -1;
}

void gpa_free(void *p)
{
    if (p)
        FreeVec(p);
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

/* Programmatic resize (PlatformWindow::resize / automation). Intuition
 * answers asynchronously with IDCMP_NEWSIZE, which drives the usual
 * renderer/GPUI resize path. */
void gpa_set_size(void *handle, int inner_w, int inner_h)
{
    GpaWindow *w = handle;
    if (!w || !w->win || inner_w < 1 || inner_h < 1)
        return;
    ChangeWindowBox(w->win, w->win->LeftEdge, w->win->TopEdge,
                    inner_w + w->win->BorderLeft + w->win->BorderRight,
                    inner_h + w->win->BorderTop + w->win->BorderBottom);
}

/* ---- Menus (gadtools NewMenu -> SetMenuStrip) --------------------------
 *
 * The Rust side flattens gpui's menu tree into a level/id/label array; this
 * builds the strip the native way: gadtools CreateMenus + LayoutMenus with
 * the screen's visual info, then SetMenuStrip. Item ids travel in
 * nm_UserData and come back on IDCMP_MENUPICK (queued, NextSelect-aware).
 * The menu bar is per window on Amiga (right mouse on the title bar), so
 * the platform applies the same spec to every window. */

typedef struct GpaMenuSpec {
    int level;           /* 0 = menu title, 1 = item, 2 = sub-item */
    int id;              /* action id; -1 = separator (label ignored) */
    const char *label;   /* system charset */
    const char *commkey; /* single-char Amiga command key or NULL */
    int disabled;
    int checked;
} GpaMenuSpec;

int gpa_set_menus(void *handle, const GpaMenuSpec *specs, int count)
{
    GpaWindow *w = handle;
    if (!w || !w->win || !GadToolsBase || count < 0)
        return -1;

    gpa_free_menus(w);
    if (count == 0)
        return 0;

    struct NewMenu *nm =
        AllocVec(sizeof(struct NewMenu) * (count + 1), MEMF_ANY | MEMF_CLEAR);
    /* Worst case two strings (label + commkey) per entry. */
    char **strs = AllocVec(sizeof(char *) * count * 2, MEMF_ANY | MEMF_CLEAR);
    if (!nm || !strs) {
        FreeVec(nm);
        FreeVec(strs);
        return -1;
    }
    int nstrs = 0;

    for (int i = 0; i < count; i++) {
        const GpaMenuSpec *s = &specs[i];
        struct NewMenu *n = &nm[i];
        n->nm_Type = (s->level <= 0) ? NM_TITLE
                     : (s->level == 1) ? NM_ITEM
                                       : NM_SUB;
        if (s->id < 0 && s->level > 0) {
            n->nm_Label = NM_BARLABEL;
        } else {
            char *copy = gpa_strdup(s->label ? s->label : "");
            if (copy)
                strs[nstrs++] = copy;
            n->nm_Label = copy ? copy : (char *)"";
        }
        if (s->commkey && s->commkey[0]) {
            char *ck = gpa_strdup(s->commkey);
            if (ck)
                strs[nstrs++] = ck;
            n->nm_CommKey = ck;
        }
        if (s->disabled)
            n->nm_Flags |= (s->level <= 0) ? NM_MENUDISABLED : NM_ITEMDISABLED;
        if (s->checked && s->level > 0)
            n->nm_Flags |= CHECKIT | CHECKED;
        n->nm_UserData = (APTR)(IPTR)s->id;
    }
    nm[count].nm_Type = NM_END;

    w->vi = GetVisualInfoA(w->win->WScreen, NULL);
    struct Menu *strip = w->vi ? CreateMenusA(nm, NULL) : NULL;
    int ok = 0;
    if (strip && LayoutMenusA(strip, w->vi, NULL)
        && SetMenuStrip(w->win, strip)) {
        w->menustrip = strip;
        w->menu_strs = strs;
        w->menu_str_count = nstrs;
        ok = 1;
    }
    FreeVec(nm);
    if (!ok) {
        if (strip)
            FreeMenus(strip);
        if (w->vi) {
            FreeVisualInfo(w->vi);
            w->vi = NULL;
        }
        for (int i = 0; i < nstrs; i++)
            FreeVec(strs[i]);
        FreeVec(strs);
        return -1;
    }
    return 0;
}

/* ---- Pointer styles (Intuition pointerclass) ---------------------------
 *
 * This AROS has no named POINTERTYPE_* set — custom pointers are 16x16,
 * 2-bitplane images on pointerclass objects. A small hand-drawn set covers
 * gpui's CursorStyle groups; style 0 restores the Intuition default via
 * ClearPointer. Objects are built lazily and shared by every window. */

enum {
    GPA_PTR_DEFAULT = 0,
    GPA_PTR_IBEAM = 1,
    GPA_PTR_CROSS = 2,
    GPA_PTR_HAND = 3,
    GPA_PTR_SIZEH = 4,
    GPA_PTR_SIZEV = 5,
    GPA_PTR_SIZED1 = 6, /* up-left / down-right */
    GPA_PTR_SIZED2 = 7, /* up-right / down-left */
    GPA_PTR_NO = 8,
    GPA_PTR_COUNT = 9,
};

/* Row-per-UWORD plane data. Plane 1 alone = outline pen, planes 1+2 = the
 * solid body pen — both visible on light and dark screens. Hotspots below. */
static UWORD gpa_ptr_img[GPA_PTR_COUNT][2][16] = {
    [GPA_PTR_IBEAM] = {
        { 0x6300, 0x1C00, 0x0800, 0x0800, 0x0800, 0x0800, 0x0800, 0x0800,
          0x0800, 0x0800, 0x0800, 0x0800, 0x0800, 0x1C00, 0x6300, 0x0000 },
        { 0x6300, 0x1C00, 0x0800, 0x0800, 0x0800, 0x0800, 0x0800, 0x0800,
          0x0800, 0x0800, 0x0800, 0x0800, 0x0800, 0x1C00, 0x6300, 0x0000 },
    },
    [GPA_PTR_CROSS] = {
        { 0x0200, 0x0200, 0x0200, 0x0200, 0x0200, 0x0200, 0xFFFC, 0x0200,
          0x0200, 0x0200, 0x0200, 0x0200, 0x0200, 0x0000, 0x0000, 0x0000 },
        { 0x0200, 0x0200, 0x0200, 0x0200, 0x0200, 0x0200, 0xFFFC, 0x0200,
          0x0200, 0x0200, 0x0200, 0x0200, 0x0200, 0x0000, 0x0000, 0x0000 },
    },
    [GPA_PTR_HAND] = {
        { 0x0C00, 0x1200, 0x1200, 0x1200, 0x13C0, 0x1278, 0x124E, 0x724A,
          0x9A4A, 0x8A4A, 0x4002, 0x2002, 0x2004, 0x1004, 0x0FF8, 0x0000 },
        { 0x0C00, 0x1E00, 0x1E00, 0x1E00, 0x1FC0, 0x1E78, 0x1FCE, 0x7FFA,
          0xFFFA, 0xFFFA, 0x7FFE, 0x3FFE, 0x3FFC, 0x1FFC, 0x0FF8, 0x0000 },
    },
    [GPA_PTR_SIZEH] = {
        { 0x0000, 0x0000, 0x0000, 0x0810, 0x1818, 0x381C, 0x7FFE, 0x381C,
          0x1818, 0x0810, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000 },
        { 0x0000, 0x0000, 0x0000, 0x0810, 0x1818, 0x381C, 0x7FFE, 0x381C,
          0x1818, 0x0810, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000 },
    },
    [GPA_PTR_SIZEV] = {
        { 0x0100, 0x0380, 0x07C0, 0x0FE0, 0x0100, 0x0100, 0x0100, 0x0100,
          0x0100, 0x0100, 0x0100, 0x0FE0, 0x07C0, 0x0380, 0x0100, 0x0000 },
        { 0x0100, 0x0380, 0x07C0, 0x0FE0, 0x0100, 0x0100, 0x0100, 0x0100,
          0x0100, 0x0100, 0x0100, 0x0FE0, 0x07C0, 0x0380, 0x0100, 0x0000 },
    },
    [GPA_PTR_SIZED1] = {
        { 0x7C00, 0x7000, 0x6800, 0x4400, 0x0200, 0x0100, 0x0080, 0x0040,
          0x0020, 0x0110, 0x008A, 0x0046, 0x002E, 0x003E, 0x0000, 0x0000 },
        { 0x7C00, 0x7000, 0x6800, 0x4400, 0x0200, 0x0100, 0x0080, 0x0040,
          0x0020, 0x0110, 0x008A, 0x0046, 0x002E, 0x003E, 0x0000, 0x0000 },
    },
    [GPA_PTR_SIZED2] = {
        { 0x003E, 0x000E, 0x0016, 0x0022, 0x0040, 0x0080, 0x0100, 0x0200,
          0x0400, 0x8800, 0x5100, 0x6200, 0x7400, 0x7C00, 0x0000, 0x0000 },
        { 0x003E, 0x000E, 0x0016, 0x0022, 0x0040, 0x0080, 0x0100, 0x0200,
          0x0400, 0x8800, 0x5100, 0x6200, 0x7400, 0x7C00, 0x0000, 0x0000 },
    },
    [GPA_PTR_NO] = {
        { 0x0FE0, 0x3018, 0x4C04, 0x8602, 0x8302, 0x8182, 0x80C2, 0x8062,
          0x4034, 0x3018, 0x0FE0, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000 },
        { 0x0FE0, 0x3FF8, 0x7C7C, 0xFE3E, 0xFF3E, 0xFFBE, 0xFFDE, 0xFFEE,
          0x7FF4, 0x3FF8, 0x0FE0, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000 },
    },
};

static const struct { int x, y; } gpa_ptr_hot[GPA_PTR_COUNT] = {
    [GPA_PTR_IBEAM] = { 4, 7 },  [GPA_PTR_CROSS] = { 6, 6 },
    [GPA_PTR_HAND] = { 5, 0 },   [GPA_PTR_SIZEH] = { 7, 6 },
    [GPA_PTR_SIZEV] = { 7, 7 },  [GPA_PTR_SIZED1] = { 7, 7 },
    [GPA_PTR_SIZED2] = { 7, 7 }, [GPA_PTR_NO] = { 5, 5 },
};

static struct BitMap gpa_ptr_bm[GPA_PTR_COUNT];
static Object *gpa_ptr_obj[GPA_PTR_COUNT];

static Object *gpa_pointer_object(int style)
{
    if (style <= GPA_PTR_DEFAULT || style >= GPA_PTR_COUNT)
        return NULL;
    if (gpa_ptr_obj[style])
        return gpa_ptr_obj[style];

    struct BitMap *bm = &gpa_ptr_bm[style];
    bm->BytesPerRow = 2;
    bm->Rows = 16;
    bm->Depth = 2;
    bm->Planes[0] = (PLANEPTR)gpa_ptr_img[style][0];
    bm->Planes[1] = (PLANEPTR)gpa_ptr_img[style][1];

    gpa_ptr_obj[style] = (Object *)NewObject(NULL, "pointerclass",
        POINTERA_BitMap, (IPTR)bm,
        POINTERA_XOffset, -gpa_ptr_hot[style].x,
        POINTERA_YOffset, -gpa_ptr_hot[style].y,
        POINTERA_WordWidth, 1,
        POINTERA_XResolution, POINTERXRESN_SCREENRES,
        POINTERA_YResolution, POINTERYRESN_SCREENRES,
        TAG_DONE);
    return gpa_ptr_obj[style];
}

void gpa_set_pointer(void *handle, int style)
{
    GpaWindow *w = handle;
    if (!w || !w->win)
        return;
    Object *obj = gpa_pointer_object(style);
    if (obj)
        SetWindowPointer(w->win, WA_Pointer, (IPTR)obj, TAG_DONE);
    else
        ClearPointer(w->win); /* default arrow (and unknown styles) */
}

/* ---- File requesters (asl.library) -------------------------------------
 *
 * Blocking, main-thread: AslRequest runs the requester's inner event loop
 * in the calling task; our window simply doesn't repaint while the (modal)
 * requester is up. Returns 0 and an AllocVec'd, NUL-separated,
 * double-NUL-terminated path list on OK; -1 on cancel/failure. */

int gpa_asl_request_paths(int save_mode, int drawers_only, int multiselect,
                          const char *initial_drawer,
                          const char *initial_file, const char *title,
                          void **out, int *out_len)
{
    *out = NULL;
    *out_len = 0;
    if (gpa_init() != 0 || !AslBase)
        return -1;

    struct FileRequester *fr = AllocAslRequestTags(ASL_FileRequest,
        ASLFR_TitleText, (IPTR)(title ? title : "Select"),
        ASLFR_InitialDrawer, (IPTR)(initial_drawer ? initial_drawer : ""),
        ASLFR_InitialFile, (IPTR)(initial_file ? initial_file : ""),
        ASLFR_DoSaveMode, save_mode ? TRUE : FALSE,
        ASLFR_DrawersOnly, drawers_only ? TRUE : FALSE,
        ASLFR_DoMultiSelect, (multiselect && !drawers_only) ? TRUE : FALSE,
        TAG_DONE);
    if (!fr)
        return -1;

    int rc = -1;
    if (AslRequestTags(fr, TAG_DONE)) {
        /* Assemble "drawer/file" paths with AddPart into one buffer. */
        ULONG cap = 4096;
        UBYTE *buf = AllocVec(cap, MEMF_ANY);
        ULONG used = 0;
        char path[1024];

        int nargs = (multiselect && fr->fr_ArgList && fr->fr_NumArgs > 0)
            ? fr->fr_NumArgs
            : 1;
        for (int i = 0; buf && i < nargs; i++) {
            const char *leaf = (nargs > 1 || (multiselect && fr->fr_ArgList))
                ? (const char *)fr->fr_ArgList[i].wa_Name
                : (const char *)fr->fr_File;
            path[0] = 0;
            if (fr->fr_Drawer) {
                strncpy(path, (const char *)fr->fr_Drawer, sizeof(path) - 1);
                path[sizeof(path) - 1] = 0;
            }
            if (!drawers_only && leaf && leaf[0]) {
                if (!AddPart((STRPTR)path, (CONST_STRPTR)leaf, sizeof(path)))
                    continue;
            }
            ULONG plen = strlen(path) + 1;
            if (used + plen + 1 > cap) {
                ULONG ncap = cap * 2 + plen;
                UBYTE *nbuf = AllocVec(ncap, MEMF_ANY);
                if (!nbuf) {
                    FreeVec(buf);
                    buf = NULL;
                    break;
                }
                memcpy(nbuf, buf, used);
                FreeVec(buf);
                buf = nbuf;
                cap = ncap;
            }
            memcpy(buf + used, path, plen);
            used += plen;
        }
        if (buf && used) {
            buf[used++] = 0; /* double-NUL terminator */
            *out = buf;
            *out_len = (int)used;
            rc = 0;
        } else {
            FreeVec(buf);
        }
    }
    FreeAslRequest(fr);
    return rc;
}
