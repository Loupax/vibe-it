//! P-FORGET-SHELL: `:forget-site [site]`.

use crate::keys::*;
use crate::protocol::Command;

fn run(k: &mut Keys, line: &str) -> Vec<Effect> {
    k.effects.clear();
    k.run_command_line(line);
    std::mem::take(&mut k.effects)
}

fn keys() -> Keys {
    Keys::new(None, "file:///run/user/1000/web-1/help.html".into(), None)
}

#[test]
fn forget_site_defaults_to_the_page_and_takes_a_site() {
    let mut k = keys();
    k.url = "https://mail.example.com/inbox".into();
    assert_eq!(
        run(&mut k, "forget-site"),
        vec![Effect::Send(Command::ForgetSite {
            site: "https://mail.example.com/inbox".into()
        })]
    );
    assert_eq!(
        run(&mut k, "forget-site other.test"),
        vec![Effect::Send(Command::ForgetSite { site: "other.test".into() })]
    );
}

#[test]
fn forget_site_refuses_without_an_http_page_and_with_extra_words() {
    let mut k = keys();
    assert!(run(&mut k, "forget-site").is_empty());
    assert!(k.message.contains("no http(s) page"), "{}", k.message);
    k.url = "file:///run/user/1000/web-1/help.html".into();
    assert!(run(&mut k, "forget-site").is_empty());
    assert!(run(&mut k, "forget-site a b").is_empty());
    assert!(k.message.contains("usage"), "{}", k.message);
    // A site given explicitly works from any page.
    assert_eq!(run(&mut k, "forget-site a.test").len(), 1);
}
