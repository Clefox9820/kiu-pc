/*
 * Widescreen, for Kid Icarus: Uprising (EUR, 0004000000030200).
 *
 * 0x0039025C returns a camera's projection matrix, the 4x4 at camera + 0x6C.
 * The first time, it asks the camera's projection object for it and copies
 * it there; after that, bit 0x4 of the word at camera + 0x8 says it is done
 * and the copy is returned as it is. Traced at run time: its instruction at
 * 0x003902B0 wrote the on-foot camera's matrix, at 0x00E14C58, once at boot,
 * and nothing wrote it again while playing.
 *
 * The matrices are turned a quarter for the 3DS's sideways screen, so the
 * second row (+0x10 to +0x1C) scales across the top screen. Halving it by
 * hand showed twice as much scene across, with the HUD unchanged.
 *
 * Zakuro leaves in the host mailbox's fifth word how much wider than the
 * console's the top screen shows, as a float: 1 for the console's 5:3,
 * 1.0667 for 16:9, 1.4 for 21:9. This divides that row by it for every
 * perspective matrix, so the game draws a wider scene into the same 400x240
 * instead of being stretched. It remembers what it applied to each camera,
 * so a shape changed during play takes effect without scaling twice.
 */
#include "overrides.h"

#define WIDENING 0x07000010u    /* Zakuro's HOST_MAILBOX_VADDR + 16 */
#define WORKED_OUT 0x4u          /* the camera's matrix is cached */
#define MINUS_ONE 0xBF800000u    /* -1.0f, a perspective matrix's w row */
#define CAMERAS 128

static struct {
    uint32_t camera;
    float scale;
} applied[CAMERAS];

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

/* the slot that remembers camera, a free one, or failing both the first.
   ponytail: linear scan of 128; a hash if the game ever has more cameras */
static int slot_of(uint32_t camera) {
    int free = -1;
    for (int i = 0; i < CAMERAS; i++) {
        if (applied[i].camera == camera) return i;
        if (free < 0 && applied[i].camera == 0) free = i;
    }
    int slot = free < 0 ? 0 : free;
    applied[slot].camera = camera;
    applied[slot].scale = 1.0f;
    return slot;
}

RECOMP_OVERRIDE(0x0039025C) {
    uint32_t camera = ctx->r[0];
    int fresh = !(mem_read32(ctx, camera + 0x8) & WORKED_OUT);
    CALL(RECOMP_ORIGINAL(0x0039025C));

    uint32_t matrix = camera + 0x6C;
    /* an orthographic matrix, for 2D, keeps its shape */
    if (mem_read32(ctx, matrix + 0x38) != MINUS_ONE) return;

    float widening = as_float(mem_read32(ctx, WIDENING));
    float scale = widening > 0.0f ? 1.0f / widening : 1.0f;
    int slot = slot_of(camera);
    if (fresh) applied[slot].scale = 1.0f; /* worked out just now, unscaled */
    if (applied[slot].scale == scale) return;

    float by = scale / applied[slot].scale;
    for (uint32_t at = matrix + 0x10; at < matrix + 0x20; at += 4) {
        mem_write32(ctx, at, as_bits(as_float(mem_read32(ctx, at)) * by));
    }
    applied[slot].scale = scale;
}
