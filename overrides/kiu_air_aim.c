/*
 * Mouse aim while flying, for Kid Icarus: Uprising (EUR, 0004000000030200).
 *
 * 0x006D2C08 runs once per tick on the reticle, right after the game moves
 * it toward the stylus, and keeps it inside the bounds it gets in s0 to s3:
 * the whole top screen while flying, a smaller box on foot. The reticle's
 * position is at r0 + 0xA4 and r0 + 0xA8, in top screen pixels. Traced at
 * run time: adding to them with the stylus up moved the reticle and it
 * stayed, and this function held it at the screen's edges.
 *
 * Zakuro leaves the mouse's movement in its host mailbox, in bottom screen
 * pixels, marked "MAM1" while it aims for flying. This moves the reticle by
 * it, scaled to the top screen the way the stylus would, before the game
 * keeps it inside the screen.
 *
 * 0x006D27E8, just before, moves the reticle toward the stylus, and with the
 * stylus up it drags the reticle along with Pit's own movement instead.
 * Traced at run time: holding the circle pad left with no touch took the
 * reticle from x 142 to 38. With the mouse aiming, the reticle stays where
 * the mouse put it, as it would under a finger, so its position is put back
 * after the game's own code runs.
 */
#include "overrides.h"

/* Zakuro's host mailbox, words of 4 bytes from its start, named and
   numbered as in zakuro/crates/zakuro-common/src/memory_map.rs */
#define HOST_MAILBOX_VADDR 0x07000000u
#define HOST_MAILBOX_LOOK_MARK 0
#define HOST_MAILBOX_LOOK_ACROSS 1
#define HOST_MAILBOX_LOOK_DOWN 2
#define HOST_MAILBOX_WORD(index) (HOST_MAILBOX_VADDR + 4u * (index))

#define MOUSE_AIM 0x314D414Du /* "MAM1" read as a little-endian word */

/* the reticle's position, in top screen pixels, as floats */
#define RETICLE_X 0xA4u
#define RETICLE_Y 0xA8u

/* top screen pixels per bottom screen pixel, 400 wide over 320 */
#define TOP_PER_BOTTOM 1.25f

static float as_float(uint32_t bits) {
    float value;
    memcpy(&value, &bits, sizeof value);
    return value;
}

static uint32_t as_bits(float value) {
    uint32_t bits;
    memcpy(&bits, &value, sizeof bits);
    return bits;
}

RECOMP_OVERRIDE(0x006D27E8) {
    if (mem_read32(ctx, HOST_MAILBOX_WORD(HOST_MAILBOX_LOOK_MARK)) != MOUSE_AIM) {
        CALL(RECOMP_ORIGINAL(0x006D27E8));
        return;
    }
    uint32_t reticle = ctx->r[0];
    uint32_t x = mem_read32(ctx, reticle + RETICLE_X);
    uint32_t y = mem_read32(ctx, reticle + RETICLE_Y);
    /* When the original yields partway (budget or svc), the rest runs as
       generated code and this tick's drift is kept. */
    CALL(RECOMP_ORIGINAL(0x006D27E8));
    mem_write32(ctx, reticle + RETICLE_X, x);
    mem_write32(ctx, reticle + RETICLE_Y, y);
}

RECOMP_OVERRIDE(0x006D2C08) {
    if (mem_read32(ctx, HOST_MAILBOX_WORD(HOST_MAILBOX_LOOK_MARK)) == MOUSE_AIM) {
        uint32_t reticle = ctx->r[0];
        float across = as_float(mem_read32(ctx, HOST_MAILBOX_WORD(HOST_MAILBOX_LOOK_ACROSS)));
        float down = as_float(mem_read32(ctx, HOST_MAILBOX_WORD(HOST_MAILBOX_LOOK_DOWN)));
        mem_write32(ctx, HOST_MAILBOX_WORD(HOST_MAILBOX_LOOK_ACROSS), 0);
        mem_write32(ctx, HOST_MAILBOX_WORD(HOST_MAILBOX_LOOK_DOWN), 0);
        float x = as_float(mem_read32(ctx, reticle + RETICLE_X)) + across * TOP_PER_BOTTOM;
        float y = as_float(mem_read32(ctx, reticle + RETICLE_Y)) + down * TOP_PER_BOTTOM;
        mem_write32(ctx, reticle + RETICLE_X, as_bits(x));
        mem_write32(ctx, reticle + RETICLE_Y, as_bits(y));
    }
    CALL(RECOMP_ORIGINAL(0x006D2C08));
}
