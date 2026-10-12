use poppy_core::CodecId;
use poppy_hw::sw::{create_software_decoder, OpenH264Decoder};
use poppy_hw::{HwDecoder, HwError};

#[test]
fn test_software_decoder_creation_and_error_handling() {
    // Unsupported codecs return UnsupportedCodec
    assert!(matches!(
        create_software_decoder(CodecId::Unknown(0x9999)),
        Err(HwError::UnsupportedCodec)
    ));

    // For H.264, if libopenh264 is not installed in the OS test environment,
    // it must return NoDeviceFound gracefully rather than crashing or returning fake frames.
    match create_software_decoder(CodecId::H264) {
        Ok(mut dec) => {
            // Real openh264 library was found on the host system!
            println!("OpenH264 library dynamically loaded successfully");
            assert!(dec.receive_frame().is_ok());
        }
        Err(HwError::NoDeviceFound) => {
            // Expected in clean headless environments without libopenh264.so
            println!("libopenh264 not present on host, graceful fallback confirmed");
        }
        Err(e) => panic!("Unexpected error creating software decoder: {e:?}"),
    }
}

#[test]
fn test_openh264_direct_instantiation() {
    match OpenH264Decoder::new() {
        Ok(mut dec) => {
            assert!(dec.flush().is_ok());
            assert!(dec.reset().is_ok());
        }
        Err(HwError::NoDeviceFound) => {
            // Graceful absent library handling
        }
        Err(e) => panic!("Unexpected error: {e:?}"),
    }
}
