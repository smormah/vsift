//! libFuzzer entry point for [`vsift_fuzz::Target::JobBatchLine`].

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    vsift_fuzz::run(vsift_fuzz::Target::JobBatchLine, data);
});
