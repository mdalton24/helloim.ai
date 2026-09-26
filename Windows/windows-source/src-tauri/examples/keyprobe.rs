//! TEMPORARY diagnostic for the 2026-08-27 disconnect bug — delete me when the
//! fix lands. Exercises the exact same keyring crate + features the app uses:
//! store, read back, delete, read back. If delete fails or the read-back after
//! delete still finds a secret, that is the disconnect bug in one screen.
fn main() {
    let e = keyring::Entry::new("NameOS", "ctest-disconnect-probe").expect("entry");
    e.set_password("dummy-secret").expect("set");
    println!("after set:    {:?}", e.get_password());
    match e.delete_credential() {
        Ok(()) => println!("delete:       Ok"),
        Err(err) => println!("delete:       ERR {err}"),
    }
    println!("after delete: {:?}", e.get_password());

    // Second shape: a FRESH Entry for the same account, the way
    // disconnect_connector builds one — in case identity differs.
    let e2 = keyring::Entry::new("NameOS", "ctest-disconnect-probe2").expect("entry2");
    e2.set_password("dummy2").expect("set2");
    let e3 = keyring::Entry::new("NameOS", "ctest-disconnect-probe2").expect("entry3");
    match e3.delete_credential() {
        Ok(()) => println!("fresh delete: Ok"),
        Err(err) => println!("fresh delete: ERR {err}"),
    }
    println!("after fresh:  {:?}", e2.get_password());
}
