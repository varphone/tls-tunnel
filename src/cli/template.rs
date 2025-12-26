use anyhow::{Context, Result};

/// Generate configuration template
pub fn generate_config_template(template_type: &str, output: Option<&str>) -> Result<()> {
    let content = match template_type {
        "server" => include_str!("../../examples/server-template.toml"),
        "client" => include_str!("../../examples/client-template.toml"),
        _ => unreachable!(),
    };

    if let Some(path) = output {
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config template to {}", path))?;
        println!(
            "Generated {} configuration template: {}",
            template_type, path
        );
    } else {
        println!("{}", content);
    }

    Ok(())
}
