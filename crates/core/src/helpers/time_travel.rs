use solana_clock::Clock;
use solana_epoch_info::EpochInfo;

use crate::types::{TimeTravelConfig, TimeTravelError};

/// Time travel math functions for calculating clock updates based on different time travel configurations.
///
/// This module contains pure functions that calculate the new clock state when time traveling
/// in a Solana network. These functions are extracted from the RPC implementation to make
/// them testable and reusable.

/// Calculates the new clock state when traveling to an absolute timestamp.
///
/// # Arguments
/// * `timestamp_target` - The target UNIX timestamp in milliseconds
/// * `current_updated_at` - The current timestamp in milliseconds
/// * `slot_time` - The slot time in milliseconds
/// * `epoch_info` - The current epoch information
///
/// # Returns
/// A `Result` containing either a new `Clock` object representing the target time state,
/// or a `TimeTravelError` if the operation is invalid
pub fn calculate_absolute_timestamp_clock(
    timestamp_target: u64,
    current_updated_at: u64,
    slot_time: u64,
    epoch_info: &EpochInfo,
) -> Result<Clock, TimeTravelError> {
    // Ensure the timestamp is in the future
    if timestamp_target < current_updated_at {
        return Err(TimeTravelError::PastTimestamp {
            target: timestamp_target,
            current: current_updated_at,
        });
    }

    // Prevent division by zero
    if slot_time == 0 {
        return Err(TimeTravelError::ZeroSlotTime);
    }

    let time_jump_in_ms = timestamp_target - current_updated_at;
    let time_jump_in_absolute_slots = time_jump_in_ms / slot_time;
    let remaining_slots_for_current_epoch = epoch_info.slots_in_epoch - epoch_info.slot_index;

    let time_jump_in_epochs = if time_jump_in_absolute_slots >= remaining_slots_for_current_epoch {
        (time_jump_in_absolute_slots - remaining_slots_for_current_epoch)
            / epoch_info.slots_in_epoch
    } else {
        0
    };

    let time_jump_in_relative_slots = if time_jump_in_epochs == 0 {
        epoch_info.slot_index + time_jump_in_absolute_slots
    } else {
        time_jump_in_absolute_slots - (time_jump_in_epochs * epoch_info.slots_in_epoch)
    };

    Ok(Clock {
        slot: time_jump_in_relative_slots,
        epoch_start_timestamp: timestamp_target as i64,
        epoch: epoch_info.epoch + time_jump_in_epochs,
        leader_schedule_epoch: 0,
        unix_timestamp: timestamp_target as i64,
    })
}

/// Calculates the new clock state when traveling to an absolute slot.
///
/// # Arguments
/// * `new_absolute_slot` - The target absolute slot number
/// * `current_absolute_slot` - The current absolute slot number
/// * `current_updated_at` - The current timestamp in milliseconds
/// * `slot_time` - The slot time in milliseconds
/// * `epoch_info` - The current epoch information
///
/// # Returns
/// A `Result` containing either a new `Clock` object representing the target slot state,
/// or a `TimeTravelError` if the operation is invalid
pub fn calculate_absolute_slot_clock(
    new_absolute_slot: u64,
    current_absolute_slot: u64,
    current_updated_at: u64,
    slot_time: u64,
    epoch_info: &EpochInfo,
) -> Result<Clock, TimeTravelError> {
    // Ensure the slot is in the future
    if new_absolute_slot < current_absolute_slot {
        return Err(TimeTravelError::PastSlot {
            target: new_absolute_slot,
            current: current_absolute_slot,
        });
    }

    let time_jump_in_absolute_slots = new_absolute_slot - current_absolute_slot;
    let time_jump_in_ms = time_jump_in_absolute_slots * slot_time;
    let timestamp_target = current_updated_at + time_jump_in_ms;
    let epoch = new_absolute_slot / epoch_info.slots_in_epoch;
    let slot = new_absolute_slot - epoch * epoch_info.slots_in_epoch;

    Ok(Clock {
        slot,
        epoch_start_timestamp: timestamp_target as i64,
        epoch,
        leader_schedule_epoch: 0,
        unix_timestamp: timestamp_target as i64,
    })
}

/// Calculates the new clock state when traveling to an absolute epoch.
///
/// # Arguments
/// * `new_epoch` - The target epoch number
/// * `current_epoch` - The current epoch number
/// * `current_absolute_slot` - The current absolute slot number
/// * `current_updated_at` - The current timestamp in milliseconds
/// * `slot_time` - The slot time in milliseconds
/// * `epoch_info` - The current epoch information
///
/// # Returns
/// A `Result` containing either a new `Clock` object representing the target epoch state,
/// or a `TimeTravelError` if the operation is invalid
pub fn calculate_absolute_epoch_clock(
    new_epoch: u64,
    current_epoch: u64,
    current_absolute_slot: u64,
    current_updated_at: u64,
    slot_time: u64,
    epoch_info: &EpochInfo,
) -> Result<Clock, TimeTravelError> {
    // Ensure the epoch is in the future
    if new_epoch < current_epoch {
        return Err(TimeTravelError::PastEpoch {
            target: new_epoch,
            current: current_epoch,
        });
    }

    let new_absolute_slot = new_epoch * epoch_info.slots_in_epoch;
    let time_jump_in_absolute_slots = if new_absolute_slot >= current_absolute_slot {
        new_absolute_slot - current_absolute_slot
    } else {
        0 // Same epoch case
    };
    let time_jump_in_ms = time_jump_in_absolute_slots * slot_time;
    let timestamp_target = current_updated_at + time_jump_in_ms;

    Ok(Clock {
        slot: 0,
        epoch_start_timestamp: timestamp_target as i64,
        epoch: new_epoch,
        leader_schedule_epoch: 0,
        unix_timestamp: timestamp_target as i64,
    })
}

/// Calculates the new clock state based on a time travel configuration.
///
/// # Arguments
/// * `config` - The time travel configuration
/// * `current_updated_at` - The current timestamp in milliseconds
/// * `slot_time` - The slot time in milliseconds
/// * `epoch_info` - The current epoch information
///
/// # Returns
/// A `Result` containing either a new `Clock` object representing the target state,
/// or a `TimeTravelError` if the operation is invalid
pub fn calculate_time_travel_clock(
    config: &TimeTravelConfig,
    current_updated_at: u64,
    slot_time: u64,
    epoch_info: &EpochInfo,
) -> Result<Clock, TimeTravelError> {
    match config {
        TimeTravelConfig::AbsoluteTimestamp(timestamp_target) => {
            calculate_absolute_timestamp_clock(
                *timestamp_target,
                current_updated_at,
                slot_time,
                epoch_info,
            )
        }
        TimeTravelConfig::AbsoluteSlot(new_absolute_slot) => calculate_absolute_slot_clock(
            *new_absolute_slot,
            epoch_info.absolute_slot,
            current_updated_at,
            slot_time,
            epoch_info,
        ),
        TimeTravelConfig::AbsoluteEpoch(new_epoch) => calculate_absolute_epoch_clock(
            *new_epoch,
            epoch_info.epoch,
            epoch_info.absolute_slot,
            current_updated_at,
            slot_time,
            epoch_info,
        ),
    }
}

#[cfg(test)]
mod tests {
    use solana_epoch_info::EpochInfo;

    use super::*;

    fn create_test_epoch_info(epoch: u64, slot_index: u64, absolute_slot: u64) -> EpochInfo {
        EpochInfo {
            epoch,
            slot_index,
            slots_in_epoch: 432_000,
            absolute_slot,
            block_height: 0,
            transaction_count: Some(0),
        }
    }

    #[test]
    fn test_calculate_absolute_timestamp_clock_basic() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000; // 1 billion ms
        let slot_time = 400; // 400ms per slot
        let target_time = current_time + 1_000_000; // 1 second later

        let clock =
            calculate_absolute_timestamp_clock(target_time, current_time, slot_time, &epoch_info)
                .unwrap();

        assert_eq!(clock.unix_timestamp, target_time as i64);
        assert_eq!(clock.epoch_start_timestamp, target_time as i64);
        assert_eq!(clock.epoch, 1); // Should stay in same epoch
        assert_eq!(clock.slot, 1000 + (1_000_000 / 400)); // Should advance by time difference
    }

    #[test]
    fn test_calculate_absolute_timestamp_clock_epoch_transition() {
        let epoch_info = create_test_epoch_info(1, 431_000, 863_000); // Near end of epoch 1
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_time = current_time + 10_000_000; // 10 seconds later

        let clock =
            calculate_absolute_timestamp_clock(target_time, current_time, slot_time, &epoch_info)
                .unwrap();

        assert_eq!(clock.unix_timestamp, target_time as i64);
        assert_eq!(clock.epoch_start_timestamp, target_time as i64);
        // With 10 seconds and 400ms slot time, we advance 25,000 slots
        // Starting from slot 431,000 in epoch 1, we should stay in epoch 1
        // since 431,000 + 25,000 = 456,000 < 864,000 (end of epoch 1)
        assert_eq!(clock.epoch, 1); // Should stay in same epoch
    }

    #[test]
    fn test_calculate_absolute_timestamp_clock_past() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_time = current_time - 1_000_000; // 1 second earlier

        let result =
            calculate_absolute_timestamp_clock(target_time, current_time, slot_time, &epoch_info);
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), TimeTravelError::PastTimestamp { target, current } if target == target_time && current == current_time)
        );
    }

    #[test]
    fn test_calculate_absolute_slot_clock_basic() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_slot = 500_000;

        let clock = calculate_absolute_slot_clock(
            target_slot,
            epoch_info.absolute_slot,
            current_time,
            slot_time,
            &epoch_info,
        )
        .unwrap();

        assert_eq!(clock.slot, target_slot % epoch_info.slots_in_epoch);
        assert_eq!(clock.epoch, target_slot / epoch_info.slots_in_epoch);
        assert_eq!(
            clock.unix_timestamp,
            (current_time + (target_slot - epoch_info.absolute_slot) * slot_time) as i64
        );
    }

    #[test]
    fn test_calculate_absolute_slot_clock_epoch_boundary() {
        let epoch_info = create_test_epoch_info(1, 431_999, 863_999); // Last slot of epoch 1
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_slot = 864_000; // First slot of epoch 2

        let clock = calculate_absolute_slot_clock(
            target_slot,
            epoch_info.absolute_slot,
            current_time,
            slot_time,
            &epoch_info,
        )
        .unwrap();

        assert_eq!(clock.slot, 0); // First slot of new epoch
        assert_eq!(clock.epoch, 2); // New epoch
        assert_eq!(clock.unix_timestamp, (current_time + slot_time) as i64);
    }

    #[test]
    fn test_calculate_absolute_slot_clock_past() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_slot = 432_000; // Earlier slot

        let result = calculate_absolute_slot_clock(
            target_slot,
            epoch_info.absolute_slot,
            current_time,
            slot_time,
            &epoch_info,
        );
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), TimeTravelError::PastSlot { target, current } if target == target_slot && current == epoch_info.absolute_slot)
        );
    }

    #[test]
    fn test_calculate_absolute_epoch_clock_basic() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_epoch = 5;

        let clock = calculate_absolute_epoch_clock(
            target_epoch,
            epoch_info.epoch,
            epoch_info.absolute_slot,
            current_time,
            slot_time,
            &epoch_info,
        )
        .unwrap();

        assert_eq!(clock.slot, 0); // Always start at slot 0 of new epoch
        assert_eq!(clock.epoch, target_epoch);
        assert_eq!(
            clock.unix_timestamp,
            (current_time
                + (target_epoch * epoch_info.slots_in_epoch - epoch_info.absolute_slot) * slot_time)
                as i64
        );
    }

    #[test]
    fn test_calculate_absolute_epoch_clock_same_epoch() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_epoch = 1; // Same epoch

        let clock = calculate_absolute_epoch_clock(
            target_epoch,
            epoch_info.epoch,
            epoch_info.absolute_slot,
            current_time,
            slot_time,
            &epoch_info,
        )
        .unwrap();

        assert_eq!(clock.slot, 0);
        assert_eq!(clock.epoch, 1);
        // When staying in the same epoch, no time should advance
        assert_eq!(clock.unix_timestamp, current_time as i64);
    }

    #[test]
    fn test_calculate_absolute_epoch_clock_past() {
        let epoch_info = create_test_epoch_info(5, 1000, 2_161_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_epoch = 1; // Earlier epoch

        let result = calculate_absolute_epoch_clock(
            target_epoch,
            epoch_info.epoch,
            epoch_info.absolute_slot,
            current_time,
            slot_time,
            &epoch_info,
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TimeTravelError::PastEpoch {
                target: 1,
                current: 5
            }
        ));
    }

    #[test]
    fn test_calculate_time_travel_clock_absolute_timestamp() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let config = TimeTravelConfig::AbsoluteTimestamp(current_time + 1_000_000);

        let clock =
            calculate_time_travel_clock(&config, current_time, slot_time, &epoch_info).unwrap();

        assert_eq!(clock.unix_timestamp, (current_time + 1_000_000) as i64);
        assert_eq!(clock.epoch, 1);
    }

    #[test]
    fn test_calculate_time_travel_clock_absolute_slot() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let config = TimeTravelConfig::AbsoluteSlot(500_000);

        let clock =
            calculate_time_travel_clock(&config, current_time, slot_time, &epoch_info).unwrap();

        assert_eq!(clock.slot, 500_000 % epoch_info.slots_in_epoch);
        assert_eq!(clock.epoch, 500_000 / epoch_info.slots_in_epoch);
    }

    #[test]
    fn test_calculate_time_travel_clock_absolute_epoch() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let config = TimeTravelConfig::AbsoluteEpoch(5);

        let clock =
            calculate_time_travel_clock(&config, current_time, slot_time, &epoch_info).unwrap();

        assert_eq!(clock.slot, 0);
        assert_eq!(clock.epoch, 5);
    }

    #[test]
    fn test_edge_case_zero_slot_time() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 0; // Edge case: zero slot time
        let target_time = current_time + 1_000_000;

        // This should return an error due to division by zero
        let result =
            calculate_absolute_timestamp_clock(target_time, current_time, slot_time, &epoch_info);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TimeTravelError::ZeroSlotTime));
    }

    #[test]
    fn test_edge_case_large_time_jump() {
        let epoch_info = create_test_epoch_info(1, 1000, 433_000);
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_time = current_time + 1_000_000_000_000; // Very large jump

        let clock =
            calculate_absolute_timestamp_clock(target_time, current_time, slot_time, &epoch_info)
                .unwrap();

        assert_eq!(clock.unix_timestamp, target_time as i64);
        assert!(clock.epoch > 1); // Should advance many epochs
    }

    #[test]
    fn test_edge_case_exact_epoch_boundary() {
        let epoch_info = create_test_epoch_info(1, 431_999, 863_999); // Last slot of epoch 1
        let current_time = 1_000_000_000;
        let slot_time = 400;
        let target_time = current_time + slot_time; // Exactly one slot later

        let clock =
            calculate_absolute_timestamp_clock(target_time, current_time, slot_time, &epoch_info)
                .unwrap();

        assert_eq!(clock.slot, 432_000); // Should advance by one slot: 431_999 + 1
        assert_eq!(clock.epoch, 1); // Should stay in same epoch since 432_000 < 864_000
    }
}
