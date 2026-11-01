//! compiles the presenter's fragment shaders to SPIR-V with naga, for
//! computers without glslc, from crates/zakuro-gpu:
//!   cargo run --release --features vulkan --example compile_present -- shaders/present.frag shaders/present.frag.spv
//!   cargo run --release --features vulkan --example compile_present -- shaders/present.frag shaders/present_stereo.frag.spv STEREO
//!   cargo run --release --features vulkan --example compile_present -- shaders/smaa_edges.frag shaders/smaa_edges.frag.spv
//!   cargo run --release --features vulkan --example compile_present -- shaders/smaa_weights.frag shaders/smaa_weights.frag.spv
//!   cargo run --release --features vulkan --example compile_present -- shaders/smaa_blend.frag shaders/smaa_blend.frag.spv
//!   cargo run --release --features vulkan --example compile_present -- shaders/rcas.frag shaders/rcas.frag.spv
//! every picture the presenter binds is a combined image sampler, which a
//! shader may take as its image and its sampler apart, at the same binding.
//! naga takes only that, and will not have both at one binding, so a shader
//! declares the sampler of the image at binding N (N from 0 to 7) at binding
//! N + 14, in any set, and it is moved onto its image's binding N here.
fn main() {
    use naga::back::spv;
    use naga::front::glsl;
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    let mut args = std::env::args().skip(1);
    let (input, output) = (args.next().unwrap(), args.next().unwrap());
    let source = std::fs::read_to_string(&input).unwrap();
    let mut options = glsl::Options::from(naga::ShaderStage::Fragment);
    for define in args {
        options.defines.insert(define, String::new());
    }
    let module = match glsl::Frontend::default().parse(&options, &source) {
        Ok(module) => module,
        Err(errors) => { eprintln!("{}", errors.emit_to_string_with_path(&source, &input)); std::process::exit(1) }
    };
    let info = match Validator::new(ValidationFlags::all(), Capabilities::all()).validate(&module) {
        Ok(info) => info,
        Err(error) => { eprintln!("{}", error.emit_to_string_with_path(&source, &input)); std::process::exit(1) }
    };
    let spv_options = spv::Options { lang_version: (1, 0), flags: spv::WriterFlags::empty(), force_loop_bounding: false, ..spv::Options::default() };
    let pipeline = spv::PipelineOptions { shader_stage: naga::ShaderStage::Fragment, entry_point: "main".into() };
    let mut words = spv::write_vec(&module, &info, &spv_options, Some(&pipeline)).unwrap();
    for binding in move_samplers(&mut words) {
        println!("moved a sampler onto binding {binding}");
    }
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    std::fs::write(&output, bytes).unwrap();
    println!("{} words", words.len());
}

/// where a shader declares the sampler of the image at binding 0.
const SAMPLER_BINDINGS: u32 = 14;
/// how many images a set may have their samplers moved onto.
const MOVED: u32 = 8;

/// rewrites every `OpDecorate %var Binding 14..=21` to the binding 14 below
/// it, giving back the bindings the samplers moved onto.
fn move_samplers(words: &mut [u32]) -> Vec<u32> {
    const OP_DECORATE: u32 = 71;
    const BINDING: u32 = 33;
    let mut moved = Vec::new();
    // the first five words are the module's header
    let mut at = 5;
    while at < words.len() {
        let (count, op) = ((words[at] >> 16) as usize, words[at] & 0xFFFF);
        if op == OP_DECORATE
            && count == 4
            && words[at + 2] == BINDING
            && (SAMPLER_BINDINGS..SAMPLER_BINDINGS + MOVED).contains(&words[at + 3])
        {
            words[at + 3] -= SAMPLER_BINDINGS;
            moved.push(words[at + 3]);
        }
        at += count.max(1);
    }
    moved
}

#[cfg(test)]
mod tests {
    /// an OpDecorate of a variable's binding, as naga writes one.
    fn binding(variable: u32, binding: u32) -> [u32; 4] {
        [(4 << 16) | 71, variable, 33, binding]
    }

    #[test]
    fn samplers_move_onto_their_images_and_nothing_else_does() {
        let header = [0x0723_0203, 0x0001_0000, 0, 100, 0];
        // a descriptor set decoration of 14 is not a binding, and stays
        let set = [(4 << 16) | 71, 9, 34, 14];
        let mut words: Vec<u32> = header.to_vec();
        for decoration in [binding(1, 0), binding(2, 14), binding(3, 1), binding(4, 15), binding(5, 16), binding(6, 21), binding(7, 22), set] {
            words.extend(decoration);
        }
        let moved = super::move_samplers(&mut words);
        assert_eq!(moved, [0, 1, 2, 7]);
        let bindings: Vec<u32> = words[5..].chunks(4).map(|decoration| decoration[3]).collect();
        assert_eq!(bindings, [0, 0, 1, 1, 2, 7, 22, 14]);
    }
}
