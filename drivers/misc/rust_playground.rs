// SPDX-License-Identifier: GPL-2.0

//! This is a Rust playground driver.

use kernel::{
    InPlaceModule, error::{Result, code}, fs::{File, Kiocb}, iov::{IovIterDest, IovIterSource}, macros::{
        module,
        vtable, //
    }, miscdevice::{
        MiscDevice,
        MiscDeviceOptions,
        MiscDeviceRegistration, //
    }, new_mutex, pr_info, prelude::*, revocable::Revocable, sync::{Arc, Mutex}, try_pin_init,
};
use pin_init::pin_data;

module! {
    type: PlaygroundModule,
    name: "rust_playground",
    authors: ["Some Where"],
    description: "Rust playground driver",
    license: "GPL v2",
}

type DeviceData = Arc<Revocable<Mutex<[u8; 256]>>>;

#[pin_data(PinnedDrop)]
struct PlaygroundModule {
    #[pin]
    device: MiscDeviceRegistration<PlaygroundDevice>,
}

impl InPlaceModule for PlaygroundModule {
    fn init(
        _module: &'static kernel::prelude::ThisModule,
    ) -> impl pin_init::PinInit<Self, kernel::error::Error> {
        let opts = MiscDeviceOptions {
            name: c"rust_playground",
        };

        pr_info!("Module init\n");

        let data_init = Arc::pin_init(Revocable::new(new_mutex!([0_u8; 256])), GFP_KERNEL);
        try_pin_init!(Self {
            device <- MiscDeviceRegistration::register(opts, data_init),
        })
    }
}

#[pinned_drop]
impl PinnedDrop for PlaygroundModule {
    fn drop(self: Pin<&mut Self>) {
        self.device.data().revoke();
        pr_info!("Module unloaded\n");
    }
}

struct PlaygroundDevice {}

#[vtable]
impl MiscDevice for PlaygroundDevice {
    type Ptr = DeviceData;
    type Data = DeviceData;

    fn open(_file: &File, misc: &MiscDeviceRegistration<Self>) -> Result<Self::Ptr> {
        Ok(misc.data().clone())
    }

    fn read_iter(mut kiocb: Kiocb<'_, Self::Ptr>, iov: &mut IovIterDest<'_>) -> Result<usize> {
        let file = kiocb.file();
        let rev_guard = file.try_access().ok_or(code::ENODEV)?;
        let guard = rev_guard.lock();
        let len = iov.simple_read_from_buffer(kiocb.ki_pos_mut(), guard.as_bytes())?;

        pr_info!("Read {} bytes\n", len);
        Ok(len)
    }

    fn write_iter(mut kiocb: Kiocb<'_, Self::Ptr>, iov: &mut IovIterSource<'_>) -> Result<usize> {
        let file = kiocb.file();
        let rev_guard = file.try_access().ok_or(code::ENODEV)?;
        let mut guard = rev_guard.lock();

        *guard = [0; 256];
        let len = iov.copy_from_iter(guard.as_mut_bytes());
        *kiocb.ki_pos_mut() = 0;

        pr_info!("Written {} bytes\n", len);
        pr_info!("Bytes {:?}\n", guard.as_ref());
        Ok(len)
    }
}
