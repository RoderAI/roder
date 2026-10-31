use serde_json::{Value, json};

use super::*;

fn fill(id: &str, label: &str, input_type: &str, form: &str) -> Value {
    json!({"id": id, "kind": "fill", "label": label, "input_type": input_type,
           "filled": false, "form": {"id": form}})
}

fn click(label: &str) -> Value {
    json!({"id": label, "kind": "click", "label": label})
}

fn page(title: &str, actions: Vec<Value>) -> Value {
    json!({"url": "https://shop.test/account/login?next=%2F", "title": title, "actions": actions})
}

#[test]
fn a_password_field_on_a_sign_in_page_is_a_password_wall() {
    let login = page(
        "Sign in",
        vec![
            fill("u", "Username", "text", "f"),
            fill("p", "Password", "password", "f"),
            click("Sign in"),
        ],
    );
    let wall = shown(&login, &[]).unwrap();
    assert_eq!(
        wall,
        JevWall {
            kind: JevWallKind::Password,
            origin: "https://shop.test".into(),
            fields: vec!["Password".into()],
        }
    );
    assert_eq!(
        serde_json::to_value(&wall).unwrap(),
        json!({"kind": "password", "origin": "https://shop.test", "fields": ["Password"]})
    );
    assert_eq!(
        wall.reason(Some("The goal gives no value.")),
        "Sign-in required at https://shop.test: the page asks for a password (\"Password\"). \
         The goal gives no value."
    );
}

#[test]
fn only_code_fields_are_a_one_time_code_wall() {
    let verify = page(
        "Verify it's you",
        vec![fill("c", "Code", "one-time-code", "f"), click("Verify")],
    );
    assert_eq!(shown(&verify, &[]).unwrap().kind, JevWallKind::OneTimeCode);
    // A code beside a password is still a password wall.
    let both = page(
        "Sign in",
        vec![
            fill("c", "Code", "one-time-code", "f"),
            fill("p", "Password", "password", "f"),
        ],
    );
    assert_eq!(shown(&both, &[]).unwrap().kind, JevWallKind::Password);
}

#[test]
fn a_page_that_names_a_sign_in_and_offers_providers_is_a_sign_in_wall() {
    let sso = page(
        "Acme",
        vec![click("Continue with Google"), click("Sign in with SSO")],
    );
    let wall = shown(&sso, &["Sign in to Acme".into()]).unwrap();
    assert_eq!(wall.kind, JevWallKind::SignIn);
    assert!(wall.fields.is_empty());
    assert_eq!(
        serde_json::to_value(&wall).unwrap(),
        json!({"kind": "sign_in", "origin": "https://shop.test"})
    );
}

#[test]
fn ordinary_pages_are_not_walls() {
    // A header's "Sign in" link on a product page.
    let product = page("Blue kettle", vec![click("Sign in"), click("Add to cart")]);
    assert_eq!(shown(&product, &["Blue kettle".into()]), None);
    // A filled password field on a page that names no sign-in, and an
    // empty one on a page that offers none.
    let mut filled = fill("p", "Password", "password", "f");
    filled["filled"] = json!(true);
    assert_eq!(
        shown(&page("Fields", vec![filled.clone(), click("Sign in")]), &[]),
        None
    );
    // On a sign-in page, a password typed and refused is still the wall.
    let refused = page("Sign in", vec![filled, click("Sign in")]);
    assert_eq!(shown(&refused, &[]).unwrap().kind, JevWallKind::Password);
    let settings = page(
        "Basic controls",
        vec![fill("s", "Secret", "password", "f"), click("Save draft")],
    );
    assert_eq!(shown(&settings, &["Basic controls".into()]), None);
    // A sign-up's new password is not a credential.
    let sign_up = page(
        "Create your account",
        vec![
            fill("n", "Create password", "password", "f"),
            click("Create account"),
            click("Log in"),
        ],
    );
    assert_eq!(shown(&sign_up, &[]), None);
    // Nor is its plain "Password" beside a confirmation.
    let sign_up = page(
        "Create your account",
        vec![
            fill("p", "Password", "password", "s"),
            fill("c", "Confirm password", "password", "s"),
            click("Create account"),
            click("Log in"),
        ],
    );
    assert_eq!(shown(&sign_up, &[]), None);
    // A heading's "Continue with" names no sign-in.
    assert_eq!(
        shown(
            &page("Checkout", vec![click("Continue with PayPal")]),
            &["Continue with checkout".into()]
        ),
        None
    );
    // A page about signing in that offers nothing to sign in with.
    assert_eq!(
        shown(&page("How to sign in", vec![click("Next article")]), &[]),
        None
    );
}

#[test]
fn a_missing_credential_or_a_field_in_its_form_is_behind_a_wall() {
    let login = page(
        "Welcome",
        vec![
            fill("u", "Email", "email", "login"),
            fill("p", "Password", "password", "login"),
            fill("q", "Search", "search", "search"),
        ],
    );
    let actions = login["actions"].as_array().unwrap();
    let wall = behind(&login, &actions[1]).unwrap();
    assert_eq!(wall.kind, JevWallKind::Password);
    assert_eq!(wall.fields, vec!["Password".to_string()]);
    // The account name in the sign-in form.
    assert_eq!(
        behind(&login, &actions[0]).unwrap().kind,
        JevWallKind::Password
    );
    // A search field outside it is a value for the caller to give.
    assert_eq!(behind(&login, &actions[2]), None);
    // A code field with no value is a code wall, wherever the page says so.
    let code = fill("c", "Code", "one-time-code", "v");
    let verify = page("Account", vec![code.clone()]);
    assert_eq!(
        behind(&verify, &code).unwrap().kind,
        JevWallKind::OneTimeCode
    );
    // A new password is the caller's value to give, not a sign-in.
    let new = fill("n", "New password", "password", "c");
    assert_eq!(behind(&page("Account", vec![new.clone()]), &new), None);
    // So is any value of a sign-up form, even its plain "Password".
    let sign_up = page(
        "Join",
        vec![
            fill("e", "Email", "email", "s"),
            fill("p", "Password", "password", "s"),
            fill("c", "Confirm password", "password", "s"),
        ],
    );
    let fields = sign_up["actions"].as_array().unwrap();
    assert_eq!(behind(&sign_up, &fields[0]), None);
    assert_eq!(behind(&sign_up, &fields[1]), None);
    // The snapshot numbers its forms: the same holds for numeric ids.
    let mut numbered = sign_up.clone();
    for field in numbered["actions"].as_array_mut().unwrap() {
        field["form"]["id"] = json!(7);
    }
    let fields = numbered["actions"].as_array().unwrap();
    assert_eq!(behind(&numbered, &fields[1]), None);
    let mut login = login.clone();
    for (field, form) in login["actions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip([3, 3, 4])
    {
        field["form"]["id"] = json!(form);
    }
    let actions = login["actions"].as_array().unwrap();
    assert_eq!(
        behind(&login, &actions[0]).unwrap().kind,
        JevWallKind::Password
    );
    assert_eq!(behind(&login, &actions[2]), None);
}

#[test]
fn a_401_wall_takes_the_page_fields_or_is_a_sign_in() {
    assert_eq!(unauthorized(&page("", vec![])).kind, JevWallKind::SignIn);
    let login = page("", vec![fill("p", "Password", "password", "f")]);
    assert_eq!(unauthorized(&login).kind, JevWallKind::Password);
}
