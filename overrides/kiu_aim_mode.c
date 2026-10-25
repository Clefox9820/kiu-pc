/*
 * Automatic mouse aim mode, for Kid Icarus: Uprising (EUR, 0004000000030200).
 *
 * Every tick the game hands the touch screen to the object playing, through
 * a virtual call at 0x00A426AC: the on-foot controller's 0x006EA18C or the
 * flying one's 0x00A3F050. Traced at run time, each ran once per tick in its
 * own kind of battle and never in the other.
 *
 * These tell Zakuro which one ran, in the host mailbox's fourth word, and
 * Zakuro switches the mouse between turning the camera and moving the
 * reticle, or lets go of it when neither runs, as in a menu.
 */
#include "overrides.h"

#define GAME_MODE 0x0700000Cu /* Zakuro's HOST_MAILBOX_VADDR + 12 */
#define GAME_ON_FOOT 1u
#define GAME_FLYING 2u

RECOMP_OVERRIDE(0x006EA18C) {
    mem_write32(ctx, GAME_MODE, GAME_ON_FOOT);
    CALL(RECOMP_ORIGINAL(0x006EA18C));
}

RECOMP_OVERRIDE(0x00A3F050) {
    mem_write32(ctx, GAME_MODE, GAME_FLYING);
    CALL(RECOMP_ORIGINAL(0x00A3F050));
}
