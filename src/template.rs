use std::collections::BTreeMap;

use anyhow::Result;
use handlebars::{Handlebars, no_escape};
use serde::Serialize;

#[derive(Serialize)]
struct TemplateContext<'a> {
    inputs: &'a BTreeMap<String, String>,
}

pub fn render(template: &str, values: &BTreeMap<String, String>) -> Result<String> {
    let mut handlebars = Handlebars::new();
    handlebars.set_strict_mode(true);
    handlebars.register_escape_fn(no_escape);
    Ok(handlebars.render_template(template, &TemplateContext { inputs: values })?)
}
