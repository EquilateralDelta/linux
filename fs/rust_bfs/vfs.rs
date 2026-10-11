use kernel::bindings;
use kernel::prelude::*;
use kernel::types::Opaque;
use pin_init::MaybeZeroable;

#[derive(MaybeZeroable)]
#[repr(transparent)]
pub(crate) struct Inode(Opaque<bindings::inode>);

impl Inode {
    pub(crate) fn as_raw(&self) -> *mut bindings::inode {
        self.0.get()
    }

    pub(crate) unsafe fn from_raw<'a>(ptr: *mut bindings::inode) -> &'a Inode {
        unsafe { &*ptr.cast() }
    }

    pub(crate) fn sb(&self) -> &SuperBlock {
        unsafe { SuperBlock::from_raw((*self.as_raw()).i_sb) }
    }

    pub(crate) fn i_mode(&self) -> u16 {
        unsafe { (*self.as_raw()).i_mode }
    }

    pub(crate) fn set_i_mode(&self, value: u16) {
        unsafe { (*self.as_raw()).i_mode = value }
    }

    pub(crate) fn set_i_op(&self, value: *const bindings::inode_operations) {
        unsafe { (*self.as_raw()).i_op = value }
    }

    pub(crate) fn set_i_fop(&self, value: *const bindings::file_operations) {
        unsafe {
            (*self.as_raw()).__bindgen_anon_3 = bindings::inode__bindgen_ty_3 { i_fop: value }
        }
    }

    pub(crate) fn set_mapping_ops(&self, value: *const bindings::address_space_operations) {
        unsafe {
            (*(*self.as_raw()).i_mapping).a_ops = value
        }
    }

    pub(crate) fn i_size(&self) -> i64 {
        unsafe { (*self.as_raw()).i_size }
    }

    pub(crate) fn set_i_size(&self, value: i64) {
        unsafe { (*self.as_raw()).i_size = value }
    }

    pub(crate) fn i_blocks(&self) -> u64 {
        unsafe { (*self.as_raw()).i_blocks }
    }

    pub(crate) fn set_i_blocks(&self, value: u64) {
        unsafe { (*self.as_raw()).i_blocks = value }
    }

    pub(crate) fn set_uid(&self, value: u32) {
        unsafe { bindings::i_uid_write(self.as_raw(), value) }
    }

    pub(crate) fn set_gid(&self, value: u32) {
        unsafe { bindings::i_gid_write(self.as_raw(), value) }
    }

    pub(crate) fn set_nlink(&self, value: u32) {
        unsafe { bindings::set_nlink(self.as_raw(), value) }
    }

    pub(crate) fn set_atime(&self, sec: i64, nsec: isize) {
        unsafe { bindings::inode_set_atime(self.as_raw(), sec, nsec) };
    }

    pub(crate) fn set_mtime(&self, sec: i64, nsec: isize) {
        unsafe { bindings::inode_set_mtime(self.as_raw(), sec, nsec) };
    }

    pub(crate) fn set_ctime(&self, sec: i64, nsec: isize) {
        unsafe { bindings::inode_set_ctime(self.as_raw(), sec, nsec) };
    }
}

#[repr(transparent)]
pub(crate) struct SuperBlock(Opaque<bindings::super_block>);

impl SuperBlock {
    pub(crate) fn as_raw(&self) -> *mut bindings::super_block {
        self.0.get()
    }

    pub(crate) unsafe fn from_raw<'a>(ptr: *mut bindings::super_block) -> &'a SuperBlock {
        unsafe { &*ptr.cast() }
    }

    /// # Safety
    ///
    /// `s_fs_info` must have been set by [`SuperBlock::set_s_fs_info`] with the same `T`, and
    /// must not be freed while the returned reference is alive.
    pub(crate) unsafe fn s_fs_info<T>(&self) -> &T {
        unsafe { &*(*self.as_raw()).s_fs_info.cast::<T>() }
    }

    /// # Safety
    ///
    /// Must be called at most once per superblock, before any call to [`SuperBlock::s_fs_info`]
    /// and while no one else can access the superblock (e.g. in `fill_super`).
    pub(crate) unsafe fn set_s_fs_info<T>(&self, value: KBox<T>) {
        unsafe { (*self.as_raw()).s_fs_info = KBox::into_raw(value).cast() }
    }

    pub(crate) fn set_s_op(&self, value: *const bindings::super_operations) {
        unsafe { (*self.as_raw()).s_op = value }
    }

    pub(crate) fn set_s_magic(&self, value: usize) {
        unsafe { (*self.as_raw()).s_magic = value }
    }

    pub(crate) fn set_s_root(&self, value: *mut bindings::dentry) {
        unsafe { (*self.as_raw()).s_root = value }
    }

    pub(crate) fn set_s_time_min(&self, value: i64) {
        unsafe { (*self.as_raw()).s_time_min = value }
    }

    pub(crate) fn set_s_time_max(&self, value: i64) {
        unsafe { (*self.as_raw()).s_time_max = value }
    }
}
