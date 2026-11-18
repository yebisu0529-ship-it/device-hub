use devhub_core::model::{ButtonAction, Change, DriverKind, KeyStroke, SettingValue};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let hub = devhub_hid::connect()?;
    let info = hub
        .scan()?
        .into_iter()
        .find(|info| info.driver == DriverKind::LogitechHidpp)
        .ok_or("PRO Wireless POWERPLAY receiver not found")?;
    let driver = hub.driver(&info.id)?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [arg] if arg == "--onboard-on" => {
            driver.run_action("onboard_enable", None)?;
        }
        [arg] if arg == "--onboard-off" => {
            driver.run_action("onboard_disable", None)?;
        }
        _ => {}
    }
    let state = driver.read()?;
    let mouse = state.mouse.ok_or("Missing mouse settings")?;
    let polling = state
        .settings
        .get("polling_wireless")
        .ok_or("Missing polling rate")?;
    if let [arg, value] = args.as_slice() {
        if arg == "--dpi" {
            let mut stage = mouse.stages[0];
            stage.x = value.parse()?;
            driver.apply(&Change::DpiStages {
                stages: vec![stage],
                active: 0,
            })?;
            println!(
                "Applied DPI: {}",
                driver.read()?.mouse.ok_or("Missing mouse settings")?.stages[0].x
            );
            return Ok(());
        }
    }
    println!("DPI: {}, polling: {:?}", mouse.stages[0].x, polling);

    println!("Stored buttons: {:?}", mouse.buttons);
    if std::env::args().any(|arg| arg == "--button-round-trip") {
        let slot = 7;
        let initial = mouse
            .buttons
            .get(slot)
            .ok_or("Missing right side button")?
            .clone();
        let temporary = ButtonAction::Key(KeyStroke {
            mods: 0,
            usage: if initial
                == ButtonAction::Key(KeyStroke {
                    mods: 0,
                    usage: 0x45,
                }) {
                0x44
            } else {
                0x45
            },
        });
        let set = |action| driver.apply(&Change::Button { slot, action });
        let verify = |expected: &ButtonAction| -> Result<(), Box<dyn std::error::Error>> {
            let actual = driver
                .read()?
                .mouse
                .ok_or("Missing mouse settings")?
                .buttons;
            let mut expected_buttons = mouse.buttons.clone();
            expected_buttons[slot] = expected.clone();
            if actual != expected_buttons {
                return Err("Stored button assignments did not match".into());
            }
            Ok(())
        };
        let change = set(temporary.clone())
            .map_err(Into::into)
            .and_then(|_| verify(&temporary));
        set(initial.clone())?;
        verify(&initial)?;
        change?;
        println!("Button round trip passed");
    }

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
