# Notices

kiu-pc is MIT licensed (see [LICENSE](LICENSE)). It carries work by others,
each under its own license:

- **Zakuro** (https://github.com/fearkov/zakuro), MIT, Copyright (c) 2026
  fearkov: `zakuro/` is a copy of it with changes made here. Its license is
  in [zakuro/LICENSE](zakuro/LICENSE).
- **Anime4K** (https://github.com/bloc97/Anime4K), MIT, Copyright (c)
  2019-2021 bloc97: its CNN shaders are in
  `zakuro/crates/zakuro-gpu/src/anime4k/` with their license header, and the
  screen filter's shaders in `zakuro/crates/zakuro-gpu/shaders/anime4k/` are
  made from them.
- **SMAA** (https://github.com/iryoku/smaa), MIT, Copyright (c) 2013 Jorge
  Jimenez, Jose I. Echevarria, Belen Masia, Fernando Navarro and Diego
  Gutierrez: `zakuro/crates/zakuro-gpu/shaders/smaa_*.frag` follow its
  passes, and `smaa_area.lut` and `smaa_search.lut` are its tables.
- **AMD FidelityFX Super Resolution 1** (https://github.com/GPUOpen-Effects/FidelityFX-FSR),
  MIT, Copyright (c) 2021 Advanced Micro Devices, Inc.: the EASU upscaling in
  `present.frag` and the RCAS sharpening in `rcas.frag` follow it.

This project is not affiliated with or endorsed by Nintendo. Kid Icarus is a
trademark of Nintendo. No part of the game is distributed with this
software: it is built on your computer from your own dump.
