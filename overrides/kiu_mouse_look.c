/*
 * Mouse look on foot, for Kid Icarus: Uprising (EUR, 0004000000030200).
 *
 * 0x0074643C runs once per game tick from the on-foot camera update. It gets
 * this tick's camera turn in s0 (yaw) and s1 (pitch), in radians, which the
 * game works out from stylus drags, and adds them to the camera's angles.
 * Found by tracing the game at run time: its only caller, at 0x0070419C,
 * loads s0 and s1 from the player's controller just before, and it ends in
 * 0x00606534, which stores yaw and pitch in the camera and points it.
 * Adding 0.05 to s0 every tick with the stylus up turned the camera
 * 1.0 rad in 20 ticks; the game itself keeps pitch within about
 * -0.19 to +0.24 rad.
 *
 * Zakuro leaves the mouse's movement in its host mailbox, in bottom screen
 * pixels, marked "MLK1" while mouse aim is set for on foot. This turns it
 * into radians the way a stylus drag would and adds it before the game's
 * own code runs.
 */
#include "overrides.h"

#define MAILBOX 0x07000000u    /* Zakuro's HOST_MAILBOX_VADDR */
#define MOUSE_LOOK 0x314B4C4Du /* "MLK1" read as a little-endian word */

/* radians per bottom screen pixel of stylus drag, measured in game:
   20 px across turned 0.0816 rad, 20 px down tilted 0.0884 rad */
#define YAW_PER_PIXEL (-0.00408f)
#define PITCH_PER_PIXEL 0.00442f

static float as_float(uint32_t bits) {
    float value;
    memcpy(&value, &bits, sizeof value);
    return value;
}

RECOMP_OVERRIDE(0x0074643C) {
    if (mem_read32(ctx, MAILBOX) == MOUSE_LOOK) {
        float across = as_float(mem_read32(ctx, MAILBOX + 4));
        float down = as_float(mem_read32(ctx, MAILBOX + 8));
        mem_write32(ctx, MAILBOX + 4, 0);
        mem_write32(ctx, MAILBOX + 8, 0);
        vfp_set_s(ctx, 0, vfp_s(ctx, 0) + across * YAW_PER_PIXEL);
        vfp_set_s(ctx, 1, vfp_s(ctx, 1) + down * PITCH_PER_PIXEL);
    }
    CALL(RECOMP_ORIGINAL(0x0074643C));
}
