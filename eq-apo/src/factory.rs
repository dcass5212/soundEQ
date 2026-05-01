// eq-apo/src/factory.rs
//
// IClassFactory implementation for SoundEqApo.
//
// COM uses a two-step activation model:
//   1. The loader calls DllGetClassObject to get an IClassFactory.
//   2. The caller calls IClassFactory::CreateInstance to get the actual object.

#![allow(non_snake_case)]

use windows::{
    core::{implement, Interface, IUnknown, Result, GUID},
    Win32::Foundation::{CLASS_E_NOAGGREGATION, BOOL},
    Win32::System::Com::{IClassFactory, IClassFactory_Impl},
};

use crate::apo::SoundEqApo;

/// COM class factory for SoundEqApo.
///
/// The `#[implement]` macro generates the vtable and ref-counting boilerplate
/// so we only need to write the interface methods.
#[implement(IClassFactory)]
pub struct ApoFactory;

impl ApoFactory {
    pub fn new() -> Self {
        ApoFactory
    }
}

impl IClassFactory_Impl for ApoFactory_Impl {
    /// Instantiate a SoundEqApo and QueryInterface it for the caller's IID.
    fn CreateInstance(
        &self,
        outer: Option<&IUnknown>,
        iid: *const GUID,
        object: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        // COM aggregation (outer != None) is not supported by this APO.
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let apo: IUnknown = SoundEqApo::new().into();
        // query = IUnknown::QueryInterface; writes the interface pointer to *object.
        unsafe { apo.query(iid, object).ok() }
    }

    /// Lock the DLL in memory. Not needed for an in-process server — the DLL
    /// stays loaded as long as live COM objects exist.
    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}
