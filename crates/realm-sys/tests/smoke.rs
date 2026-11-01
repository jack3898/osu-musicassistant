use std::ffi::CStr;

#[test]
fn links_against_realm_core() {
    let version = unsafe { CStr::from_ptr(realm_sys::realm_get_library_version()) };
    assert_eq!(version.to_str().unwrap(), "20.1.2");
}
