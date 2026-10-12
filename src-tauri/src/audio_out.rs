use crate::audio_toolkit::audio::output_device_name;
use log::{debug, warn};
use rodio::cpal::traits::HostTrait;
use rodio::cpal::BufferSize;
use rodio::{DeviceSinkBuilder, MixerDeviceSink};

/// Opens the output device named in settings, falling back to the system
/// default when it is unset, `"Default"`, or no longer connected.
pub fn open_output(settings_device: Option<&str>) -> Result<MixerDeviceSink, String> {
    let sink_builder = match settings_device {
        Some(device_name) if device_name != "Default" => {
            let host = crate::audio_toolkit::get_cpal_host();
            let found_device = host
                .output_devices()
                .map_err(|e| format!("Failed to list output devices: {}", e))?
                .find(|device| output_device_name(device).as_deref() == Some(device_name));

            match found_device {
                Some(device) => DeviceSinkBuilder::from_device(device),
                None => {
                    warn!("Device '{}' not found, using default device", device_name);
                    DeviceSinkBuilder::from_default_device()
                }
            }
        }
        _ => {
            debug!("Using default device");
            DeviceSinkBuilder::from_default_device()
        }
    }
    .map_err(|e| format!("Failed to select output device: {}", e))?;

    // The builder asks for a fixed ~50 ms buffer, which some devices reject
    // (Bluetooth headsets cap at 1024 frames); let the device choose instead.
    sink_builder
        .with_buffer_size(BufferSize::Default)
        .open_stream()
        .map_err(|e| format!("Failed to open output stream: {}", e))
}
