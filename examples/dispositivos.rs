//! Lista los dispositivos de audio y cuáles son los predeterminados para cada
//! papel (normal, multimedia y comunicaciones). cargo run --example dispositivos

use wasapi::{DeviceEnumerator, Direction, Role, initialize_mta};

fn main() -> anyhow::Result<()> {
    initialize_mta().ok()?;
    let enumerator = DeviceEnumerator::new()?;
    for (direction, label) in [(Direction::Capture, "MICRÓFONOS"), (Direction::Render, "ALTAVOCES")] {
        println!("{label}");
        let collection = enumerator.get_device_collection(direction)?;
        for i in 0..collection.get_nbr_devices()? {
            let device = collection.get_device_at_index(i)?;
            println!("  - {}", device.get_friendlyname()?);
        }
        for role in [Role::Console, Role::Multimedia, Role::Communications] {
            let name = enumerator.get_default_device_for_role(direction, role).and_then(|d| d.get_friendlyname());
            println!("  predeterminado {role:?}: {}", name.unwrap_or_else(|e| format!("(ninguno: {e})")));
        }
    }
    Ok(())
}
