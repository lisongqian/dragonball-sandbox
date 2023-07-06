// Copyright 2022 Alibaba Cloud. All Rights Reserved.
//
// Copyright 2018 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Portions Copyright 2017 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the THIRD-PARTY file.
use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;

use log::debug;
use log::trace;
use log::warn;
use virtio_queue::QueueT;
use vm_memory::GuestAddressSpace;
use vm_memory::GuestMemoryRegion;

use dbs_device::resources::ResourceConstraint;
use dbs_utils::epoll_manager::{EpollManager, SubscriberId};
use dbs_utils::metric::IncMetric;

use crate::device::{VirtioDeviceConfig, VirtioDeviceInfo};
use crate::vsock::metrics::VsockDeviceMetrics;
use crate::{ActivateResult, DbsGuestAddressSpace, VirtioDevice};

use super::backend::VsockBackend;
use super::defs::uapi;
use super::epoll_handler::VsockEpollHandler;
use super::muxer::{Error as MuxerError, VsockGenericMuxer, VsockMuxer};
use super::{Result, VsockError};

const VSOCK_DRIVER_NAME: &str = "virtio-vsock";
const VSOCK_CONFIG_SPACE_SIZE: usize = 8;
const VSOCK_AVAIL_FEATURES: u64 =
    1u64 << uapi::VIRTIO_F_VERSION_1 | 1u64 << uapi::VIRTIO_F_IN_ORDER;

/// This is the `VirtioDevice` implementation for our vsock device. It handles
/// the virtio-level device logic: feature negociation, device configuration,
/// and device activation. The run-time device logic (i.e. event-driven data
/// handling) is implemented by `super::epoll_handler::EpollHandler`.
///
/// The vsock device has two input parameters: a CID to identify the device, and
/// a `VsockBackend` to use for offloading vsock traffic.
///
/// Upon its activation, the vsock device creates its `EpollHandler`, passes it
/// the event-interested file descriptors, and registers these descriptors with
/// the VMM `EpollContext`. Going forward, the `EpollHandler` will get notified
/// whenever an event occurs on the just-registered FDs:
/// - an RX queue FD;
/// - a TX queue FD;
/// - an event queue FD; and
/// - a backend FD.
pub struct Vsock<AS: GuestAddressSpace, M: VsockGenericMuxer = VsockMuxer> {
    cid: u64,
    queue_sizes: Arc<Vec<u16>>,
    device_info: VirtioDeviceInfo,
    subscriber_id: Option<SubscriberId>,
    muxer: Option<M>,
    phantom: PhantomData<AS>,
    metrics: Arc<VsockDeviceMetrics>,
}

// Default muxer implementation of Vsock
impl<AS: GuestAddressSpace> Vsock<AS> {
    /// Create a new virtio-vsock device with the given VM CID and vsock
    /// backend.
    pub fn new(cid: u64, queue_sizes: Arc<Vec<u16>>, epoll_mgr: EpollManager) -> Result<Self> {
        let muxer = VsockMuxer::new(cid).map_err(VsockError::Muxer)?;
        let metrics = muxer.metrics.clone();
        Self::new_with_muxer(cid, queue_sizes, epoll_mgr, muxer, metrics)
    }
}

impl<AS: GuestAddressSpace, M: VsockGenericMuxer> Vsock<AS, M> {
    pub(crate) fn new_with_muxer(
        cid: u64,
        queue_sizes: Arc<Vec<u16>>,
        epoll_mgr: EpollManager,
        muxer: M,
        metrics: Arc<VsockDeviceMetrics>,
    ) -> Result<Self> {
        let mut config_space = Vec::with_capacity(VSOCK_CONFIG_SPACE_SIZE);
        for i in 0..VSOCK_CONFIG_SPACE_SIZE {
            config_space.push((cid >> (8 * i as u64)) as u8);
        }

        Ok(Vsock {
            cid,
            queue_sizes: queue_sizes.clone(),
            device_info: VirtioDeviceInfo::new(
                VSOCK_DRIVER_NAME.to_string(),
                VSOCK_AVAIL_FEATURES,
                queue_sizes,
                config_space,
                epoll_mgr,
            ),
            subscriber_id: None,
            muxer: Some(muxer),
            phantom: PhantomData,
            metrics,
        })
    }

    fn id(&self) -> &str {
        &self.device_info.driver_name
    }

    /// add backend for vsock muxer
    // NOTE: Backend is not allowed to add when vsock device is activated.
    pub fn add_backend(&mut self, backend: Box<dyn VsockBackend>, is_default: bool) -> Result<()> {
        if let Some(muxer) = self.muxer.as_mut() {
            muxer
                .add_backend(backend, is_default)
                .map_err(VsockError::Muxer)
        } else {
            Err(VsockError::Muxer(MuxerError::BackendAddAfterActivated))
        }
    }

    pub fn get_metrics(&self) -> Arc<VsockDeviceMetrics> {
        self.metrics.clone()
    }
}

impl<AS, Q, R, M> VirtioDevice<AS, Q, R> for Vsock<AS, M>
where
    AS: DbsGuestAddressSpace,
    Q: QueueT + Send + 'static,
    R: GuestMemoryRegion + Sync + Send + 'static,
    M: VsockGenericMuxer + 'static,
{
    fn device_type(&self) -> u32 {
        uapi::VIRTIO_ID_VSOCK
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &self.queue_sizes
    }

    fn get_avail_features(&self, page: u32) -> u32 {
        self.device_info.get_avail_features(page)
    }

    fn set_acked_features(&mut self, page: u32, value: u32) {
        trace!(target: "virtio-vsock", "{}: VirtioDevice::set_acked_features({}, 0x{:x})",
            self.id(), page, value
        );
        self.device_info.set_acked_features(page, value)
    }

    fn read_config(&mut self, offset: u64, data: &mut [u8]) {
        trace!(target: "virtio-vsock", "{}: VirtioDevice::read_config(0x{:x}, {:?})",
            self.id(), offset, data);
        if !self.device_info.read_config(offset, data) {
            self.metrics.cfg_fails.inc();
        }
    }

    fn write_config(&mut self, offset: u64, data: &[u8]) {
        trace!(target: "virtio-vsock", "{}: VirtioDevice::write_config(0x{:x}, {:?})",
        self.id(), offset, data);
        if !self.device_info.write_config(offset, data) {
            self.metrics.cfg_fails.inc();
        }
    }

    fn activate(&mut self, config: VirtioDeviceConfig<AS, Q, R>) -> ActivateResult {
        trace!(target: "virtio-vsock", "{}: VirtioDevice::activate()", self.id());

        self.device_info
            .check_queue_sizes(&config.queues[..])
            .map_err(|e| {
                self.metrics.activate_fails.inc();
                e
            })?;
        let handler: VsockEpollHandler<AS, Q, R, M> = VsockEpollHandler::new(
            config,
            self.id().to_owned(),
            self.cid,
            // safe to unwrap, because we create muxer using New()
            self.muxer.take().unwrap(),
            self.metrics.clone(),
        );

        self.subscriber_id = Some(self.device_info.register_event_handler(Box::new(handler)));

        Ok(())
    }

    fn get_resource_requirements(
        &self,
        requests: &mut Vec<ResourceConstraint>,
        use_generic_irq: bool,
    ) {
        trace!(target: "virtio-vsock", "{}: VirtioDevice::get_resource_requirements()", self.id());

        requests.push(ResourceConstraint::LegacyIrq { irq: None });
        if use_generic_irq {
            requests.push(ResourceConstraint::GenericIrq {
                size: (self.queue_sizes.len() + 1) as u32,
            });
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn remove(&mut self) {
        let subscriber_id = self.subscriber_id.take();
        if let Some(subscriber_id) = subscriber_id {
            match self.device_info.remove_event_handler(subscriber_id) {
                Ok(_) => debug!("virtio-vsock: removed subscriber_id {:?}", subscriber_id),
                Err(err) => warn!("virtio-vsock: failed to remove event handler: {:?}", err),
            };
        } else {
            self.muxer.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use kvm_ioctls::Kvm;
    use virtio_queue::QueueSync;
    use vm_memory::{GuestAddress, GuestMemoryMmap, GuestRegionMmap};

    use dbs_device::resources::DeviceResources;
    use dbs_interrupt::NoopNotifier;

    use crate::device::VirtioDeviceConfig;
    use crate::VirtioQueueConfig;

    use super::super::defs::uapi;
    use super::super::tests::{test_bytes, TestContext};
    use super::*;

    impl<AS: DbsGuestAddressSpace, M: VsockGenericMuxer + 'static> Vsock<AS, M> {
        pub fn mock_activate(
            &mut self,
            config: VirtioDeviceConfig<AS, QueueSync, GuestRegionMmap>,
        ) -> Result<VsockEpollHandler<AS, QueueSync, GuestRegionMmap, M>> {
            trace!(target: "virtio-vsock", "{}: VirtioDevice::activate_re()", self.id());

            self.device_info
                .check_queue_sizes(&config.queues[..])
                .unwrap();
            let handler: VsockEpollHandler<AS, QueueSync, GuestRegionMmap, M> =
                VsockEpollHandler::new(
                    config,
                    self.id().to_owned(),
                    self.cid,
                    // safe to unwrap, because we create muxer using New()
                    self.muxer.take().unwrap(),
                    self.metrics.clone(),
                );

            Ok(handler)
        }
    }

    #[test]
    fn test_virtio_device() {
        let mut ctx = TestContext::new();
        let device_features = VSOCK_AVAIL_FEATURES;
        let driver_features: u64 = VSOCK_AVAIL_FEATURES | 1 | (1 << 32);
        let device_pages = [
            (device_features & 0xffff_ffff) as u32,
            (device_features >> 32) as u32,
        ];
        let driver_pages = [
            (driver_features & 0xffff_ffff) as u32,
            (driver_features >> 32) as u32,
        ];
        assert_eq!(
            VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::device_type(
                &ctx.device
            ),
            uapi::VIRTIO_ID_VSOCK
        );
        assert_eq!(
            VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::get_avail_features(
                &ctx.device, 0
            ),
            device_pages[0]
        );
        assert_eq!(
            VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::get_avail_features(
                &ctx.device, 1
            ),
            device_pages[1]
        );
        assert_eq!(
            VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::get_avail_features(
                &ctx.device, 2
            ),
            0
        );

        // Ack device features, page 0.
        ctx.device
            .device_info
            .set_acked_features(0, driver_pages[0]);
        // Ack device features, page 1.
        ctx.device
            .device_info
            .set_acked_features(1, driver_pages[1]);
        // Ack some bogus page (i.e. 2). This should have no side effect.
        ctx.device.device_info.set_acked_features(2, 0);
        // Attempt to un-ack the first feature page. This should have no side effect.
        ctx.device
            .device_info
            .set_acked_features(0, !driver_pages[0]);
        // Check that no side effect are present, and that the acked features are exactly the same
        // as the device features.
        assert_eq!(
            ctx.device.device_info.acked_features(),
            device_features & driver_features
        );

        // Test reading 32-bit chunks.
        let mut data = [0u8; 8];
        VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::read_config(
            &mut ctx.device,
            0,
            &mut data[..4],
        );
        test_bytes(&data[..], &(ctx.cid & 0xffff_ffff).to_le_bytes());
        VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::read_config(
            &mut ctx.device,
            4,
            &mut data[4..],
        );
        test_bytes(&data[4..], &((ctx.cid >> 32) & 0xffff_ffff).to_le_bytes());

        // Test reading 64-bit.
        let mut data = [0u8; 8];
        VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::read_config(
            &mut ctx.device,
            0,
            &mut data,
        );
        test_bytes(&data, &ctx.cid.to_le_bytes());

        // Check out-of-bounds reading.
        let mut data = [0u8, 1, 2, 3, 4, 5, 6, 7];
        VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::read_config(
            &mut ctx.device,
            2,
            &mut data,
        );
        assert_eq!(data, [0u8, 0, 0, 0, 0, 0, 6, 7]);

        // Just covering lines here, since the vsock device has no writable config.
        // A warning is, however, logged, if the guest driver attempts to write any config data.
        VirtioDevice::<Arc<GuestMemoryMmap<()>>, QueueSync, GuestRegionMmap>::write_config(
            &mut ctx.device,
            0,
            &data[..4],
        );

        let mem = GuestMemoryMmap::<()>::from_ranges(&[(GuestAddress(0), 0x10000)]).unwrap();
        let queues = vec![
            VirtioQueueConfig::<QueueSync>::create(2, 0).unwrap(),
            VirtioQueueConfig::<QueueSync>::create(2, 0).unwrap(),
            VirtioQueueConfig::<QueueSync>::create(2, 0).unwrap(),
        ];
        let kvm = Kvm::new().unwrap();
        let vm_fd = Arc::new(kvm.create_vm().unwrap());
        let resources = DeviceResources::new();
        let config = VirtioDeviceConfig::<Arc<GuestMemoryMmap<()>>>::new(
            Arc::new(mem),
            vm_fd,
            resources,
            queues,
            None,
            Arc::new(NoopNotifier::new()),
        );

        // Test activation.
        ctx.device.activate(config).unwrap();
    }
}
