/* smoke_main.c -- C harness for the GPUI-on-AROS smoke (mirrors
 * hosted/rust/rs3_main.c): C owns AROS startup, hands argc/argv to the
 * rust-aros std, and calls the Rust entry that runs the GPUI app.
 */
#include <proto/dos.h> /* PutStr -- no <stdio.h> */

#define GPUI_SMOKE_MAGIC 0x47505541u /* "GPUA" */

extern unsigned int aros_gpui_smoke_main(void);

/* Read by std sys/args/aros.rs so std::env::args() works (C owns main). */
int aros_argc = 0;
char **aros_argv = 0;

int main(int argc, char **argv)
{
    unsigned int rc;
    aros_argc = argc;
    aros_argv = argv;
    PutStr("[GPUI-SMOKE] starting run loop (close the window to exit)\n");
    rc = aros_gpui_smoke_main();
    if (rc == GPUI_SMOKE_MAGIC) {
        PutStr("[GPUI-SMOKE] clean exit PASS\n");
        return 0;
    }
    PutStr("[GPUI-SMOKE] FAIL\n");
    return 20;
}
