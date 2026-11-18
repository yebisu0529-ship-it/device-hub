use devhub_core::model::{Change, DriverKind, SettingValue};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let hub = devhub_hid::connect()?;
    let info = hub
        .scan()?
        .into_iter()
        .find(|info| info.driver == DriverKind::LogitechHidpp)
        .ok_or("PRO Wireless POWERPLAY receiver not found")?;
    let driver = hub.driver(&info.id)?;
    let state = driver.read()?;
    let mouse = state.mouse.ok_or("Missing mouse settings")?;
    let polling = state
        .settings
        .get("polling_wireless")
        .ok_or("Missing polling rate")?;
    println!("DPI: {}, polling: {:?}", mouse.stages[0].x, polling);

    if std::env::args().any(|arg| arg == "--round-trip") {
        let mut stage = mouse.stages[0];
        stage.x = if stage.x == 800 { 850 } else { 800 };
        let dpi_change = |stage| Change::DpiStages {
            stages: vec![stage],
            active: 0,
        };
        let change = driver.apply(&dpi_change(stage));
        let restore = driver.apply(&dpi_change(mouse.stages[0]));
        restore?;
        change?;
        println!("DPI round trip passed");

        let rate_change = |value| Change::Setting {
            id: "polling_wireless".into(),
            value,
        };
        let test_rate = if polling.as_choice() == 500 {
            1000
        } else {
            500
        };
        let change = driver.apply(&rate_change(SettingValue::Choice(test_rate)));
        let restore = driver.apply(&rate_change(polling.clone()));
        restore?;
        change?;
        println!("Polling round trip passed");
    }
    Ok(())
}
