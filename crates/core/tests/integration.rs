use std::time::Duration;

use crossbeam_channel::unbounded;
use surfpool_core::{
    simnet::{start, SimnetEvent},
    types::{RunloopTriggerMode, SimnetConfig, SurfpoolConfig},
};

#[tokio::test]
async fn test_simnet_ready() {
    let config = SurfpoolConfig {
        simnet: SimnetConfig {
            runloop_trigger_mode: RunloopTriggerMode::Manual, // Prevent ticks
            ..SimnetConfig::default()
        },
        ..SurfpoolConfig::default()
    };

    let (simnet_events_tx, simnet_events_rx) = unbounded();
    let (_simnet_commands_tx, simnet_commands_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start(&config, simnet_events_tx, simnet_commands_rx);
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    match simnet_events_rx.recv() {
        Ok(SimnetEvent::Ready) => println!("Simnet is ready"),
        e => panic!("Expected Ready event: {e:?}"),
    }
}

#[tokio::test]
async fn test_simnet_ticks() {
    let config = SurfpoolConfig {
        simnet: SimnetConfig {
            slot_time: 1,
            ..SimnetConfig::default()
        },
        ..SurfpoolConfig::default()
    };

    let (simnet_events_tx, simnet_events_rx) = unbounded();
    let (_simnet_commands_tx, simnet_commands_rx) = unbounded();
    let (test_tx, test_rx) = unbounded();

    let _handle = hiro_system_kit::thread_named("test").spawn(move || {
        let future = start(&config, simnet_events_tx, simnet_commands_rx);
        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            panic!("{e:?}");
        }
    });

    let _ = hiro_system_kit::thread_named("ticks").spawn(move || {
        let mut ticks = 0;
        loop {
            match simnet_events_rx.recv() {
                Ok(SimnetEvent::ClockUpdate(_)) => ticks += 1,
                _ => (),
            }

            if ticks > 100 {
                let _ = test_tx.send(true);
            }
        }
    });

    match test_rx.recv_timeout(Duration::from_millis(1000)) {
        Ok(_) => (),
        Err(_) => panic!("not enough ticks"),
    }
}
