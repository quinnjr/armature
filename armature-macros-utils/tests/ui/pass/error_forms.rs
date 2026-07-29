// Every documented form of the error-handling macros must compile.
use armature_core::Error;
use armature_macros_utils::{bail, ensure};

fn ensures(cond: bool, id: i64) -> Result<(), Error> {
    // ensure!: condition + message, condition + kind + message,
    // condition + kind + format args.
    ensure!(cond, "User account is inactive");
    ensure!(id >= 0, BadRequest, "Must be non-negative");
    ensure!(id >= 0, NotFound, "record {} missing", id);
    Ok(())
}

fn bails(id: i64) -> Result<(), Error> {
    // bail!: bare message, and kind + format args.
    if id < 0 {
        bail!("User not found");
    }
    if id == 0 {
        bail!(NotFound, "User {} not found", id);
    }
    Ok(())
}

fn main() {
    let _ = ensures(true, 1);
    let _ = bails(1);
}
