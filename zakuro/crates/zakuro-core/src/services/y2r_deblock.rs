//! the smoothing of a compressed video's 8 by 8 blocks where they meet,
//! which Y2R's planes go through before they become pixels.

/// the side of a compressed video's block, in samples.
const BLOCK: usize = 8;
/// the most a block's edge steps by; a bigger step is the picture's own edge.
const STEP: i32 = 32;
/// the most the samples on either side of a seam vary from one to the next;
/// a side that varies more is texture, not a flat block.
const FLAT: i32 = 10;
/// the samples looked at across a seam, half a block on each side, whose
/// ends the ramp runs between.
const WINDOW: usize = BLOCK;

/// smooths a frame's planes, Y and its U and V at half the width, and at
/// half the lines too when each chroma line serves two.
pub(super) fn deblock_planes(planes: &mut [Vec<u8>; 3], width: usize, lines: usize, lines_per_chroma: usize) {
    let [luma, chroma_u, chroma_v] = planes;
    deblock(luma, width, lines);
    deblock(chroma_u, width / 2, lines / lines_per_chroma);
    deblock(chroma_v, width / 2, lines / lines_per_chroma);
}

/// smooths where a compressed video's 8 by 8 blocks meet, across and down
/// a plane of samples width to a row: a seam whose sides are both flat and
/// whose step is small is a block's edge, not the picture's, and becomes a
/// ramp over the four samples on each side, half a block; any other is
/// left alone.
// one fixed strength, where a decoder's loop filter would follow the
// stream's quantiser, which Y2R does not see.
pub(super) fn deblock(plane: &mut [u8], width: usize, lines: usize) {
    if plane.len() < width * lines {
        return;
    }
    let offsets = |step: usize| std::array::from_fn::<isize, WINDOW, _>(|k| (k as isize - (WINDOW / 2) as isize) * step as isize);
    for row in (0..lines).map(|y| y * width) {
        for x in (BLOCK..width.saturating_sub(WINDOW / 2 - 1)).step_by(BLOCK) {
            seam(plane, offsets(1).map(|d| (row + x).wrapping_add_signed(d)));
        }
    }
    for y in (BLOCK..lines.saturating_sub(WINDOW / 2 - 1)).step_by(BLOCK) {
        for x in 0..width {
            seam(plane, offsets(width).map(|d| (y * width + x).wrapping_add_signed(d)));
        }
    }
}

/// one seam, the window of samples at `at` across it, made a ramp between
/// the window's ends when it is a block's edge.
fn seam(plane: &mut [u8], at: [usize; WINDOW]) {
    let v = at.map(|i| plane[i] as i32);
    let half = WINDOW / 2;
    let flat = v[..half].windows(2).chain(v[half..].windows(2)).all(|pair| (pair[0] - pair[1]).abs() < FLAT);
    if !flat || (v[half - 1] - v[half]).abs() >= STEP {
        return;
    }
    let last = (WINDOW - 1) as i32;
    for (k, &i) in at.iter().enumerate().take(WINDOW - 1).skip(1) {
        let k = k as i32;
        plane[i] = ((v[0] * (last - k) + v[WINDOW - 1] * k + last / 2) / last) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a compressed video's blocks: a step where two flat 8 by 8 blocks
    /// meet is smoothed into a ramp, across the plane and down it; a real
    /// edge, a step that is not at a block's side and a textured side stay
    /// as they are.
    #[test]
    fn video_blocks_are_smoothed_where_they_meet() {
        let plane = |left: u8, right: u8| -> Vec<u8> { (0..16).map(|x| if x < 8 { left } else { right }).collect::<Vec<u8>>().repeat(8) };
        let mut flat = plane(100, 112);
        deblock(&mut flat, 16, 8);
        let line = flat[..16].to_vec();
        assert!(line.windows(2).all(|w| w[0] <= w[1]), "a ramp, {line:?}");
        assert!(line[8] - line[7] < 6, "no step left at the seam, {line:?}");
        assert!(flat.chunks(16).all(|row| row == line), "every row alike");
        // a bigger step between flat blocks, as a dark scene's are, goes too,
        // over a wider ramp
        let mut bigger = plane(100, 128);
        deblock(&mut bigger, 16, 8);
        let line = bigger[..16].to_vec();
        assert!(line.windows(2).all(|w| w[0] <= w[1]) && line[8] - line[7] < 6, "a bigger step smoothed, {line:?}");
        assert!(line[5] > 100 && line[10] < 128, "over four samples on each side, {line:?}");
        let mut edge = plane(20, 200);
        deblock(&mut edge, 16, 8);
        assert_eq!(edge, plane(20, 200), "a real edge stays");
        let mut off: Vec<u8> = (0..16).map(|x| if x < 5 { 100 } else { 112 }).collect::<Vec<u8>>().repeat(8);
        let before = off.clone();
        deblock(&mut off, 16, 8);
        assert_eq!(off, before, "a step off the block's side stays");
        let mut textured: Vec<u8> = (0..16).map(|x| if x % 2 == 0 { 90 } else { 120 }).collect::<Vec<u8>>().repeat(8);
        let before = textured.clone();
        deblock(&mut textured, 16, 8);
        assert_eq!(textured, before, "texture stays");
        // where a row of blocks meets the one below
        let mut tall: Vec<u8> = (0..16).flat_map(|y| [if y < 8 { 100u8 } else { 112 }; 8]).collect();
        deblock(&mut tall, 8, 16);
        let column: Vec<u8> = (0..16).map(|y| tall[y * 8]).collect();
        assert!(column.windows(2).all(|w| w[0] <= w[1]) && column[8] - column[7] < 6, "no step down the seam, {column:?}");
    }

    /// the planes of a 4:2:0 frame: the colour planes are smoothed at half
    /// the width and half the lines, so a seam down them at their own eighth
    /// line goes, as the luma's does at its eighth.
    #[test]
    fn a_frames_colour_planes_are_smoothed_at_their_own_size() {
        let (width, lines) = (16, 16);
        let tall = |width: usize, lines: usize| -> Vec<u8> { (0..lines).flat_map(|y| vec![if y < 8 { 100u8 } else { 112 }; width]).collect() };
        let mut planes = [tall(width, lines), tall(width / 2, lines), tall(width / 2, lines)];
        deblock_planes(&mut planes, width, lines, 2);
        let column = |plane: &[u8], width: usize, lines: usize| -> Vec<u8> { (0..lines).map(|y| plane[y * width]).collect() };
        let luma = column(&planes[0], width, lines);
        assert!(luma.windows(2).all(|w| w[0] <= w[1]) && luma[8] - luma[7] < 6, "the luma's seam smoothed, {luma:?}");
        // the colour planes have 8 lines here, no seam inside them
        assert_eq!(planes[1], tall(width / 2, lines), "half the lines, no seam to smooth");
        assert_eq!(planes[2], tall(width / 2, lines));
    }
}
