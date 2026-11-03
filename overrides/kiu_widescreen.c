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
 * so a shape changed during play takes effect without scaling twice. Past
 * 128 cameras, one it no longer remembers keeps the scale it has.
 */
#include "overrides.h"

/* Zakuro's host mailbox, words of 4 bytes from its start, named and
   numbered as in zakuro/crates/zakuro-common/src/memory_map.rs */
#define HOST_MAILBOX_VADDR 0x07000000u
#define HOST_MAILBOX_TOP_WIDENING 4
#define HOST_MAILBOX_WORD(index) (HOST_MAILBOX_VADDR + 4u * (index))

/* the camera, by byte offset */
#define CAMERA_FLAGS 0x8u        /* WORKED_OUT says the matrix is cached */
#define CAMERA_PROJECTION 0x6Cu  /* the 4x4 projection matrix, as floats */
#define WORKED_OUT 0x4u

/* the projection matrix, by byte offset */
#define MATRIX_ACROSS_ROW 0x10u  /* the second row, across the top screen */
#define MATRIX_ROW_SIZE 0x10u
#define MATRIX_W_ROW_Z 0x38u     /* MINUS_ONE in a perspective matrix */
#define MINUS_ONE 0xBF800000u    /* -1.0f */

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

/* the next slot a fresh camera takes once every slot is in use */
static int reused;

/* the slot that remembers camera, or a free one for it. Slots are never
   freed, since nothing here sees a camera go away, so once all are in use a
   camera missing from them may be one that lost its slot with its matrix
   already scaled. Only a matrix worked out just now is known to be unscaled:
   a fresh camera takes the slots in turn, and any other gets -1 and is left
   as it is, rather than scaled twice. The scan is linear, which is enough
   for the cameras the game has. */
static int slot_of(uint32_t camera, int fresh) {
    int free = -1;
    for (int i = 0; i < CAMERAS; i++) {
        if (applied[i].camera == camera) return i;
        if (free < 0 && applied[i].camera == 0) free = i;
    }
    if (free < 0) {
        if (!fresh) return -1;
        free = reused;
        reused = (reused + 1) % CAMERAS;
    }
    applied[free].camera = camera;
    applied[free].scale = 1.0f;
    return free;
}

RECOMP_OVERRIDE(0x0039025C) {
    uint32_t camera = ctx->r[0];
    int fresh = !(mem_read32(ctx, camera + CAMERA_FLAGS) & WORKED_OUT);
    CALL(RECOMP_ORIGINAL(0x0039025C));

    uint32_t matrix = camera + CAMERA_PROJECTION;
    /* an orthographic matrix, for 2D, keeps its shape */
    if (mem_read32(ctx, matrix + MATRIX_W_ROW_Z) != MINUS_ONE) return;

    float widening = as_float(mem_read32(ctx, HOST_MAILBOX_WORD(HOST_MAILBOX_TOP_WIDENING)));
    float scale = widening > 0.0f ? 1.0f / widening : 1.0f;
    int slot = slot_of(camera, fresh);
    if (slot < 0) return;
    if (fresh) applied[slot].scale = 1.0f; /* worked out just now, unscaled */
    if (applied[slot].scale == scale) return;

    float by = scale / applied[slot].scale;
    uint32_t row = matrix + MATRIX_ACROSS_ROW;
    for (uint32_t at = row; at < row + MATRIX_ROW_SIZE; at += 4) {
        mem_write32(ctx, at, as_bits(as_float(mem_read32(ctx, at)) * by));
    }
    applied[slot].scale = scale;
}
