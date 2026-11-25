// SPDX-License-Identifier: GPL-3.0-or-later

/// Translated text.
pub fn tr(msgid: &str) -> String {
    gettextrs::gettext(msgid)
}

/// Translated text with `{}` placeholders filled in order.
pub fn trf(msgid: &str, args: &[&str]) -> String {
    let mut out = gettextrs::gettext(msgid);
    for arg in args {
        if let Some(pos) = out.find("{}") {
            out.replace_range(pos..pos + 2, arg);
        }
    }
    out
}

/// Singular or plural form, `{}` replaced by `n`.
pub fn trn(singular: &str, plural: &str, n: usize) -> String {
    gettextrs::ngettext(singular, plural, n as u32).replace("{}", &n.to_string())
}

/// Like `trf`, with a context for texts that are the same in English but not in other languages.
pub fn trcf(context: &str, msgid: &str, args: &[&str]) -> String {
    let mut out = gettextrs::pgettext(context, msgid);
    for arg in args {
        if let Some(pos) = out.find("{}") {
            out.replace_range(pos..pos + 2, arg);
        }
    }
    out
}
