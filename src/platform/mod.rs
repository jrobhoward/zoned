#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "freebsd")]
mod freebsd;

#[cfg(target_os = "linux")]
pub(crate) use linux::PlatformDevice;

#[cfg(target_os = "freebsd")]
pub(crate) use freebsd::PlatformDevice;

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub(crate) struct PlatformDevice;

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
impl PlatformDevice {
    pub(crate) fn open(_path: &std::path::Path) -> crate::error::Result<Self> {
        Err(crate::error::ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn device_info(&self) -> crate::error::Result<crate::types::DeviceInfo> {
        Err(crate::error::ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn report_zones(
        &self,
        _sector: u64,
        _max_zones: u32,
    ) -> crate::error::Result<Vec<crate::types::Zone>> {
        Err(crate::error::ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn reset_zones(&self, _sector: u64, _nr_sectors: u64) -> crate::error::Result<()> {
        Err(crate::error::ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn open_zones(&self, _sector: u64, _nr_sectors: u64) -> crate::error::Result<()> {
        Err(crate::error::ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn close_zones(&self, _sector: u64, _nr_sectors: u64) -> crate::error::Result<()> {
        Err(crate::error::ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn finish_zones(&self, _sector: u64, _nr_sectors: u64) -> crate::error::Result<()> {
        Err(crate::error::ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        std::path::Path::new("")
    }
}
