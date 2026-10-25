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

#define MAILBOX 0x07000000u   /* Zakuro's HOST_MAILBOX_VADDR */
#define MOUSE_AIM 0x314D414Du /* "MAM1" read as a little-endian word */

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
    if (mem_read32(ctx, MAILBOX) != MOUSE_AIM) {
        CALL(RECOMP_ORIGINAL(0x006D27E8));
        return;
    }
    uint32_t reticle = ctx->r[0];
    uint32_t x = mem_read32(ctx, reticle + 0xA4);
    uint32_t y = mem_read32(ctx, reticle + 0xA8);
    /* ponytail: when the original yields partway (budget or svc), the rest
       runs as generated code and this tick's drift is kept */
    CALL(RECOMP_ORIGINAL(0x006D27E8));
    mem_write32(ctx, reticle + 0xA4, x);
    mem_write32(ctx, reticle + 0xA8, y);
}

RECOMP_OVERRIDE(0x006D2C08) {
    if (mem_read32(ctx, MAILBOX) == MOUSE_AIM) {
        uint32_t reticle = ctx->r[0];
        float across = as_float(mem_read32(ctx, MAILBOX + 4));
        float down = as_float(mem_read32(ctx, MAILBOX + 8));
        mem_write32(ctx, MAILBOX + 4, 0);
        mem_write32(ctx, MAILBOX + 8, 0);
        float x = as_float(mem_read32(ctx, reticle + 0xA4)) + across * TOP_PER_BOTTOM;
        float y = as_float(mem_read32(ctx, reticle + 0xA8)) + down * TOP_PER_BOTTOM;
        mem_write32(ctx, reticle + 0xA4, as_bits(x));
        mem_write32(ctx, reticle + 0xA8, as_bits(y));
    }
    CALL(RECOMP_ORIGINAL(0x006D2C08));
}
