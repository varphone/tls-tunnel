use anyhow::Result;

/// Register as systemd service (Linux only)
pub fn register_systemd_service(
    _service_type: &str,
    _config: &str,
    _name: Option<&str>,
    _exec: Option<&str>,
) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Service registration is only supported on Linux");
    }

    #[cfg(target_os = "linux")]
    {
        use anyhow::Context;
        use std::process::Command;

        let service_name = _name.unwrap_or_else(|| match _service_type {
            "server" => "tls-tunnel-server",
            "client" => "tls-tunnel-client",
            _ => unreachable!(),
        });

        let exec_path = if let Some(custom) = _exec {
            custom.to_string()
        } else {
            std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "tls-tunnel".to_string())
        };

        let config_path = std::fs::canonicalize(_config)
            .with_context(|| format!("Failed to resolve config path: {}", _config))?
            .to_string_lossy()
            .into_owned();

        let description = match _service_type {
            "server" => "TLS Tunnel Server",
            "client" => "TLS Tunnel Client",
            _ => unreachable!(),
        };

        let unit_content = format!(
            "[Unit]\n\
            Description={}\n\
            After=network-online.target\n\
            Wants=network-online.target\n\
            \n\
            [Service]\n\
            Type=simple\n\
            ExecStart={} {} --config {}\n\
            Restart=on-failure\n\
            RestartSec=3\n\
            Environment=RUST_LOG=info\n\
            \n\
            [Install]\n\
            WantedBy=multi-user.target\n",
            description, exec_path, _service_type, config_path
        );

        let unit_file = format!("/etc/systemd/system/{}.service", service_name);

        // Write systemd unit file
        std::fs::write(&unit_file, unit_content)
            .with_context(|| format!("Failed to write systemd unit file to {}", unit_file))?;

        println!("✓ Created systemd service file: {}", unit_file);

        // Reload systemd daemon
        let output = Command::new("systemctl")
            .arg("daemon-reload")
            .output()
            .context("Failed to reload systemd daemon")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to reload systemd daemon: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        println!("✓ Reloaded systemd daemon");

        // Enable service
        let output = Command::new("systemctl")
            .arg("enable")
            .arg(format!("{}.service", service_name))
            .output()
            .context("Failed to enable service")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to enable service: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        println!("✓ Enabled {} service", service_name);
        println!("\nService registered successfully!");
        println!("Start service: sudo systemctl start {}", service_name);
        println!("Check status:  sudo systemctl status {}", service_name);
        println!("View logs:     sudo journalctl -u {} -f", service_name);

        Ok(())
    }
}

/// Unregister systemd service (Linux only)
pub fn unregister_systemd_service(_service_type: &str, _name: Option<&str>) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Service unregistration is only supported on Linux");
    }

    #[cfg(target_os = "linux")]
    {
        use anyhow::Context;
        use std::path::Path;
        use std::process::Command;

        let service_name = _name.unwrap_or_else(|| match _service_type {
            "server" => "tls-tunnel-server",
            "client" => "tls-tunnel-client",
            _ => unreachable!(),
        });

        let unit_file = format!("/etc/systemd/system/{}.service", service_name);

        // Check if service exists
        if !Path::new(&unit_file).exists() {
            anyhow::bail!("Service {} not found", service_name);
        }

        // Stop service if running
        let output = Command::new("systemctl")
            .arg("stop")
            .arg(format!("{}.service", service_name))
            .output()
            .context("Failed to stop service")?;

        if output.status.success() {
            println!("✓ Stopped {} service", service_name);
        }

        // Disable service
        let output = Command::new("systemctl")
            .arg("disable")
            .arg(format!("{}.service", service_name))
            .output()
            .context("Failed to disable service")?;

        if output.status.success() {
            println!("✓ Disabled {} service", service_name);
        }

        // Remove service file
        std::fs::remove_file(&unit_file)
            .with_context(|| format!("Failed to remove service file: {}", unit_file))?;

        println!("✓ Removed service file: {}", unit_file);

        // Reload systemd daemon
        let output = Command::new("systemctl")
            .arg("daemon-reload")
            .output()
            .context("Failed to reload systemd daemon")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to reload systemd daemon: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        println!("✓ Reloaded systemd daemon");
        println!("\nService unregistered successfully!");

        Ok(())
    }
}
