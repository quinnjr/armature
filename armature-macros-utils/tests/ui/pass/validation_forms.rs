// Every documented form of the validation macros must compile.
use armature_core::Error;
use armature_macros_utils::{validate, validate_email, validate_required};

fn is_valid_email(value: &str) -> bool {
    value.contains('@')
}

fn run(age: i64, email: &str, name: Option<String>, token: Option<String>) -> Result<(), Error> {
    // validate!: condition + message, and custom-function form.
    validate!(age >= 18, "Must be 18 or older");
    validate!(email.len() >= 3, "Email too short");
    validate!(email, is_valid_email, "Invalid email format");

    // validate_required!: single and multiple fields.
    validate_required!(name);
    validate_required!(name, token);

    // validate_email!: single field.
    validate_email!(email);

    Ok(())
}

fn main() {
    let _ = run(20, "a@b.com", Some("x".into()), Some("y".into()));
}
