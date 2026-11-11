use std::io::{self, Write};

use crate::{
	config::{self, Color, Dpi, MouseButtonType, RangedByte},
	error,
};

/// List of USB devices to look for `(<vendor id>, <product id>)`.
static TARGET_DEVICES: &[(u16, u16)] = &[
	(0x258a, 0x0033), // Model D
	(0x258a, 0x0036), // Model O
];

/// Vendor-specific HID usage page used by the configuration interface.
#[cfg(target_os = "macos")]
const VENDOR_USAGE_PAGE: u16 = 0xff00;

/// Builds a packet matching the `Type 1` section of the `Main Packet`
/// section of `packet_spec.md`. This packet controls DPI values, DPI states,
/// DPI colors, selected DPI, liftoff distance, and RGB.
///
/// # Panics
///
/// If there is an error writing to the command array (probably won't happen),
/// or the selected DPI is not enabled (should be checked before `config` is
/// passed here), the program will exit with an error message.
fn build_main_packet(config: &config::Config) -> [u8; 520] {
	let mut data = io::Cursor::new([0u8; 520]);
	macro_rules! write {
		($buf:expr, [$($data:tt)*]) => {
			$buf.write_all(&[$($data)*]).unwrap_or_else(|e| error!("error writing usb command to buffer: {e}"))
		};
		[$($data:tt)*] => {
			write!(data, [$($data)*])
		};
	}

	let write_color =
		|data: &mut io::Cursor<[u8; 520]>, &Color { r, g, b }| write!(data, [r, b, g]);
	let combine_brightness_speed =
		|brightness: &RangedByte<1, 4>, speed: &RangedByte<1, 3>| (**brightness << 4) | **speed;

	// unknown data
	#[rustfmt::skip]
	write![0x04, 0x11, 0x00, 0x7b, 0x00, 0x00, 0x00, 0x00, 0x64, 0x06];

	let separate_xy_dpi = config
		.dpi
		.iter()
		.any(|Dpi { x_dpi, y_dpi, .. }| x_dpi != y_dpi);

	write![(if separate_xy_dpi { 0x08 } else { 0x00 }) | (config.polling_rate as u8)];

	// dpi enable flags
	let mut dpi_count = 0u8;
	let mut current_dpi = Option::<u8>::None;
	let mut flags = 0u8;

	for (i, flag) in config
		.dpi
		.iter()
		.enumerate()
		.filter(|(_, Dpi { enable, .. })| *enable)
		.map(|(i, _)| (i, 0b1 << i))
	{
		dpi_count += 1;
		if *config.current_dpi as usize == i {
			// current dpi is index (by 1) in the list of enabled dpis
			current_dpi = Some(dpi_count);
		}
		flags |= flag;
	}
	flags = !flags;

	let current_dpi = match current_dpi {
		Some(current_dpi) => current_dpi,
		None => error!("selected DPI is not enabled"),
	};

	write![(current_dpi << 4) | dpi_count, flags];

	if separate_xy_dpi {
		for Dpi { x_dpi, y_dpi, .. } in &config.dpi {
			write![*x_dpi, *y_dpi];
		}
	} else {
		for Dpi { x_dpi: dpi, .. } in &config.dpi {
			write![*dpi];
		}
		write![0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
	}

	// unknown data
	write![0x00, 0x00, 0x00, 0x00];

	// DPI colors
	for Dpi {
		color: Color { r, g, b },
		..
	} in &config.dpi
	{
		write![*r, *g, *b];
	}

	#[rustfmt::skip]
	write![
		// unknown data
		0x00, 0x00, 0x00, 0x00, 0x00,  0x00,
		
		// LED mode
		config.lighting.mode as u8,
		
		// Rainbow speed
		0x40 | *config.lighting.rainbow.speed,
		// Rainbow direction
		config.lighting.rainbow.direction as u8,
		
		// Solid brightness
		*config.lighting.solid.brightness << 4,
	];

	// Solid color
	write_color(&mut data, &config.lighting.solid.color);

	// unknown data
	write![0x42, 0x07];

	// Breathing colors
	for color in &config.lighting.breathing.colors {
		write_color(&mut data, color);
	}

	#[rustfmt::skip]
	write![
		// Tail brightness / speed
		combine_brightness_speed(
			&config.lighting.tail.brightness, 
			&config.lighting.tail.speed
		),
		
		// Fade speed
		0x40 | *config.lighting.fade.speed,
		
		// unknown data (probably tail/fade colors or some combination thereof)
		                                                       0x00, 0xff, 0x00, 0x00, 0x00, 0xff, 0x00,
		0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0xff,  0xff, 0xff, 0xff, 0xff, 0xfa, 0x00, 0xff, 0xff,
		0x00, 0x00, 0xff, 0x00, 0x00, 0xff, 0x00, 0x00,
		
		// Rave brightness / speed
		combine_brightness_speed(
			&config.lighting.rave.brightness,
			&config.lighting.rave.speed,
		),
	];

	for color in &config.lighting.rave.colors {
		write_color(&mut data, color);
	}

	#[rustfmt::skip]
	write![
		// unknown data
		0x02,
		
		// Wave brightness / speed
		combine_brightness_speed(
			&config.lighting.wave.brightness,
			&config.lighting.wave.speed,
		),
		
		// Breathing (Single) speed
		*config.lighting.breathing_single.speed,
	];

	// Breathing (Single) color
	write_color(&mut data, &config.lighting.breathing_single.color);

	// Liftoff distance
	write![config.liftoff_distance as u8];

	data.into_inner()
}

/// Builds a packet matching the `Type 2` section of the `Main Packet` section
/// of `packet_spec.md`. This packet controls mouse button actions.
///
/// # Panics
///
/// If there is an error writing to the command array (probably won't happen)
fn build_buttons_packet(config: &config::Config) -> [u8; 520] {
	let mut data = io::Cursor::new([0u8; 520]);
	macro_rules! write {
		($buf:expr, [$($data:tt)*]) => {
			$buf.write_all(&[$($data)*]).unwrap_or_else(|e| error!("error writing usb command to buffer: {e}"))
		};
		[$($data:tt)*] => {
			write!(data, [$($data)*])
		};
	}

	let write_button = |data: &mut io::Cursor<[u8; 520]>, button: &config::MouseButtonType| {
		data.write_all(&u32::to_be_bytes(*button as u32))
			.unwrap_or_else(|e| error!("error writing usb command to buffer: {e}"));
	};

	// unknown data
	write![0x04, 0x12, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00];

	write_button(&mut data, &config.buttons.left);
	write_button(&mut data, &config.buttons.right);
	write_button(&mut data, &config.buttons.middle);
	write_button(&mut data, &config.buttons.back);
	write_button(&mut data, &config.buttons.forward);
	write_button(&mut data, &config.buttons.dpi);

	for _ in 0..13 {
		write_button(&mut data, &MouseButtonType::Disable);
	}

	data.into_inner()
}

/// Builds a packet matching the `Debounce Packet` section of `packet_spec.md`.
/// This packet controls debounce time.
#[rustfmt::skip]
fn build_debounce_packet(config: &config::Config) -> [u8; 6] {
	[
		0x05, 0x1a, // unknown data
		config.debounce_time as u8,
		0x00, 0x00, 0x00,
	]
}

fn access_denied_hint(context: &str) -> ! {
	#[cfg(target_os = "macos")]
	error!(
		"{context}\n\
		hint: macOS blocks this because the mouse config interface also exposes a keyboard HID collection.\n\
		      sudo alone is not enough — grant Input Monitoring, then fully quit/reopen the terminal:\n\
		      1. System Settings → Privacy & Security → Input Monitoring\n\
		      2. Enable your terminal app (Terminal, Warp, Cursor, etc.) — use + if it is missing\n\
		      3. Quit that app completely (Cmd+Q) and reopen it\n\
		      4. Run: sudo $(which glorious-mouse-control) ..."
	);
	#[cfg(not(target_os = "macos"))]
	error!(
		"{context}\n\
		hint: you may need to run as root, or grant write access to your mouse (udev rules)"
	);
}

/// Applies the specified `config` to a connected target device.
///
/// # Panics
///
/// See [`build_main_packet`]
pub fn apply_config(config: &config::Config) {
	let main_packet = build_main_packet(config);
	let buttons_packet = build_buttons_packet(config);
	let debounce_packet = build_debounce_packet(config);

	send_packets(&main_packet, &buttons_packet, &debounce_packet);
}

#[cfg(target_os = "macos")]
fn send_packets(main_packet: &[u8], buttons_packet: &[u8], debounce_packet: &[u8]) {
	use hidapi::HidApi;

	let api = HidApi::new().unwrap_or_else(|e| error!("could not initialize hidapi: {e}"));
	// Shared mode: do not seize the pointer/keyboard from the system HID stack.
	api.set_open_exclusive(false);

	// Prefer the vendor collection on interface 1 (matches USB wIndex=0x1).
	// Note: on this mouse, interface 1 also includes a keyboard usage, so macOS
	// gates IOHIDDeviceOpen behind Input Monitoring for the calling terminal.
	let info = api
		.device_list()
		.find(|device| {
			let id = (device.vendor_id(), device.product_id());
			TARGET_DEVICES.iter().any(|target| &id == target)
				&& device.interface_number() == 1
				&& device.usage_page() == VENDOR_USAGE_PAGE
		})
		.or_else(|| {
			api.device_list().find(|device| {
				let id = (device.vendor_id(), device.product_id());
				TARGET_DEVICES.iter().any(|target| &id == target) && device.interface_number() == 1
			})
		})
		.unwrap_or_else(|| error!("could not find usb device"));

	let device = info.open_device(&api).unwrap_or_else(|e| {
		let msg = e.to_string();
		if msg.contains("not permitted") || msg.contains("Access") || msg.contains("Permission") {
			access_denied_hint(&format!("could not open hid device: {e}"));
		}
		error!("could not open hid device: {e}")
	});

	for (label, packet) in [
		("main", main_packet),
		("buttons", buttons_packet),
		("debounce", debounce_packet),
	] {
		device.send_feature_report(packet).unwrap_or_else(|e| {
			let msg = e.to_string();
			if msg.contains("not permitted") || msg.contains("Access") || msg.contains("Permission") {
				access_denied_hint(&format!("could not send {label} feature report: {e}"));
			}
			error!("could not send {label} feature report: {e}")
		});
	}
}

#[cfg(not(target_os = "macos"))]
mod libusb_backend {
	use std::{ops::Deref, time::Duration};

	use rusb::{Device, DeviceHandle};

	use super::{TARGET_DEVICES, access_denied_hint};
	use crate::error;

	/// Finds a device matching one of [`TARGET_DEVICES`].
	fn find_device() -> Device<rusb::GlobalContext> {
		rusb::devices()
			.unwrap_or_else(|e| error!("could not load usb device list: {e}"))
			.iter()
			.find(|device| {
				let descriptor = device
					.device_descriptor()
					.unwrap_or_else(|e| error!("could not get device descriptor: {e}"));
				let id = (descriptor.vendor_id(), descriptor.product_id());
				TARGET_DEVICES.iter().any(|target| &id == target)
			})
			.unwrap_or_else(|| error!("could not find usb device"))
	}

	/// Manages claiming and release of usb device interfaces. (claimed
	/// interfaces will be released and reattached to the kernel (if applicable)
	/// once this struct is dropped)
	struct InterfaceScopeWrapper<'h, const N: usize> {
		interfaces: [(u8, bool); N],
		handle: &'h mut DeviceHandle<rusb::GlobalContext>,
	}

	impl<'h, const N: usize> InterfaceScopeWrapper<'h, N> {
		fn wrap(handle: &'h mut DeviceHandle<rusb::GlobalContext>, interfaces: [u8; N]) -> Self {
			Self {
				interfaces: interfaces.map(|interface| {
					let attached = rusb::supports_detach_kernel_driver()
						&& handle
							.kernel_driver_active(interface)
							.unwrap_or_else(|e| error!("could not query kernel driver on interface {interface}: {e}"));
					if attached {
						handle.detach_kernel_driver(interface).unwrap_or_else(|e| {
							error!("could not detach kernel driver from interface {interface}: {e}")
						});
					}
					handle.claim_interface(interface).unwrap_or_else(|e| {
						if matches!(e, rusb::Error::Access) {
							access_denied_hint(&format!(
								"could not claim usb interface {interface}: {e}"
							));
						}
						error!("could not claim usb interface {interface}: {e}")
					});

					(interface, attached)
				}),
				handle,
			}
		}
	}

	impl<'h, const N: usize> Drop for InterfaceScopeWrapper<'h, N> {
		fn drop(&mut self) {
			for (interface, attached) in self.interfaces {
				let _ = self.handle.release_interface(interface);
				if attached {
					let _ = self.handle.attach_kernel_driver(interface);
				}
			}
		}
	}

	impl<'h, const N: usize> Deref for InterfaceScopeWrapper<'h, N> {
		type Target = DeviceHandle<rusb::GlobalContext>;

		fn deref(&self) -> &Self::Target {
			self.handle
		}
	}

	pub fn send_packets(main_packet: &[u8], buttons_packet: &[u8], debounce_packet: &[u8]) {
		let device = find_device();

		let mut handle = device.open().unwrap_or_else(|e| {
			if matches!(e, rusb::Error::Access) {
				access_denied_hint(&format!("could not open usb device: {e}"));
			}
			error!("could not open usb device: {e}")
		});

		// Only interface 1 is used by the HID Set_Report control transfers
		// (wIndex = 0x1). Claiming interface 0 is unnecessary.
		let handle = InterfaceScopeWrapper::wrap(&mut handle, [1]);

		handle
			.write_control(0x21, 0x09, 0x0304, 0x1, main_packet, Duration::from_secs(5))
			.unwrap_or_else(|e| error!("could not send main packet: {e}"));
		handle
			.write_control(0x21, 0x09, 0x0304, 0x1, buttons_packet, Duration::from_secs(5))
			.unwrap_or_else(|e| error!("could not send buttons packet: {e}"));
		handle
			.write_control(0x21, 0x09, 0x0305, 0x1, debounce_packet, Duration::from_secs(5))
			.unwrap_or_else(|e| error!("could not send debounce packet: {e}"));
	}
}

#[cfg(not(target_os = "macos"))]
fn send_packets(main_packet: &[u8], buttons_packet: &[u8], debounce_packet: &[u8]) {
	libusb_backend::send_packets(main_packet, buttons_packet, debounce_packet);
}
