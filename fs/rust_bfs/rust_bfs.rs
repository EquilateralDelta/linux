// SPDX-License-Identifier: GPL-2.0

//! This is a Rust Bfs driver.

#![allow(dead_code)]

// TODO refactor to remote some of AS conversions

use core::ops::{Deref, Range};

use kernel::bitmap::BitmapVec;
use kernel::error::{from_result, to_result};
use kernel::sync::Mutex;
use kernel::types::ForeignOwnable;
use kernel::{
    bindings,
    error::{code, Error, Result},
    types::Opaque,
    InPlaceModule,
};
use kernel::{new_mutex, prelude::*};
use vfs::{Inode, KStatFs, SuperBlock};

mod vfs;

const BFS_BSIZE_BITS: u32 = 9;
const BFS_BSIZE: u32 = 1 << BFS_BSIZE_BITS;
const BFS_MAGIC: u32 = 0x1BADFACE;
const BFS_INODES_PER_BLOCK: u32 = 8;
const BFS_DIRENT_SIZE: u32 = 16;
const BFS_INODE_SIZE: u32 = 64;
const BFS_NAMELEN: u32 = 14;

const BFS_VDIR: u32 = 2;
const BFS_VREG: u32 = 1;

module! {
    type: RustBfs,
    name: "rust_bfs",
    authors: ["Rust for Linux Contributors"],
    description: "Rust boot file system",
    license: "GPL",
}

#[pin_data(PinnedDrop)]
struct RustBfs {
    #[pin]
    fst: Opaque<bindings::file_system_type>,
}

// SAFETY: `register_filesystem` and `unregister_filesystem` fully acquire and
// release `file_systems_lock` within their own call, with no thread-affine
// state held across calls — safe to call from any thread, and register/
// unregister need not happen on the same one.
unsafe impl Send for RustBfs {}
// SAFETY: `register_filesystem`/`unregister_filesystem` take `file_systems_lock`
// internally, so concurrent calls from multiple threads are synchronized
// kernel-side.
unsafe impl Sync for RustBfs {}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Ino(u16);

impl Ino {
    const UNUSED: Ino = Ino(0);
    const ROOT: Ino = Ino(2);
    /// Highest inode number a BFS filesystem can have.
    const MAX: Ino = Ino(513);

    fn from_inode(inode: &Inode) -> Self {
        Ino(inode.i_ino() as u16)
    }

    fn num(self) -> u16 {
        self.0
    }

    fn is_unused(self) -> bool {
        self == Self::UNUSED
    }

    fn block(self) -> u32 {
        1 + (self.num() - Self::ROOT.num()) as u32 / BFS_INODES_PER_BLOCK
    }

    fn block_range(self) -> Range<usize> {
        let idx = (self.num() - Self::ROOT.num()) as usize % BFS_INODES_PER_BLOCK as usize;
        idx * (BFS_INODE_SIZE as usize)..(idx + 1) * (BFS_INODE_SIZE as usize)
    }

    fn validate(self, sb: &SuperBlock) -> Result<()> {
        let sb_info: &BfsSbInfo = unsafe { sb.s_fs_info() };
        if (Self::ROOT..=sb_info.last_inode).contains(&self) {
            Ok(())
        } else {
            Err(code::EIO)
        }
    }
}

impl From<Ino> for u64 {
    fn from(ino: Ino) -> u64 {
        ino.num() as u64
    }
}

impl From<Ino> for usize {
    fn from(ino: Ino) -> usize {
        ino.num() as usize
    }
}

#[pin_data]
struct BfsSbInfo {
    total_blocks: u32,
    free_blocks: u32,
    free_inodes: u32,
    end_block: u32,
    last_inode: Ino,
    inodes_bitmap: BitmapVec,
    #[pin]
    bfs_lock: Mutex<()>,
}

struct BfsInodeInfo {
    dsk_ino: Ino,
    sblock: u32,
    eblock: u32,
    // struct mapping_metadata_bhs i_metadata_bhs;
    vfs_inode: Inode,
}

struct BufferHead(*mut bindings::buffer_head);

impl BufferHead {
    fn read(sb: &SuperBlock, block: u32) -> Result<Self> {
        let res = unsafe { bindings::sb_bread(sb.as_raw(), block as u64) };

        if res.is_null() {
            Err(code::EIO)
        } else {
            Ok(Self(res))
        }
    }

    fn read_header_block(sb: &SuperBlock) -> Result<Self> {
        Self::read(sb, 0)
    }

    // SAFETY: block shouldn't me modified for the duration of the slice lifetime
    unsafe fn data(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts((*self.0).b_data, (*self.0).b_size) }
    }

    // SAFETY: slice shouldn't me modified for the duration of the slice lifetime
    unsafe fn data_slice(&self, range: Range<usize>) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts((*self.0).b_data.wrapping_add(range.start), range.len())
        }
    }

    // SAFETY: slice should have exclusive access to the data of the block for the slice lifetime, usually through bfs_lock mutex
    unsafe fn data_slice_mut(&self, range: Range<usize>) -> &mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut((*self.0).b_data.wrapping_add(range.start), range.len())
        }
    }
}

impl Drop for BufferHead {
    fn drop(&mut self) {
        unsafe { bindings::brelse(self.0) };
    }
}

fn take_n_bytes<const N: usize>(buf: &mut &[u8]) -> [u8; N] {
    let res = &buf[..N];
    *buf = &buf[N..];

    return res.try_into().unwrap();
}

fn take_u32(buf: &mut &[u8]) -> u32 {
    u32::from_le_bytes(take_n_bytes(buf))
}

fn take_u16(buf: &mut &[u8]) -> u16 {
    u16::from_le_bytes(take_n_bytes(buf))
}

fn write_n_bytes<const N: usize>(buf: &mut &mut [u8], value: [u8; N]) {
    let dest = &mut buf[..N];
    dest.copy_from_slice(&value);
    *buf = &mut core::mem::take(buf)[N..];
}

fn write_u32(buf: &mut &mut [u8], value: u32) {
    write_n_bytes(buf, value.to_le_bytes());
}

fn write_u16(buf: &mut &mut [u8], value: u16) {
    write_n_bytes(buf, value.to_le_bytes());
}

struct BfsSuperBlock {
    magic: u32,
    start: u32,
    end: u32,
    from: u32,
    to: u32,
}

impl BfsSuperBlock {
    fn read(sb: &SuperBlock) -> Result<Self> {
        let bh = BufferHead::read_header_block(sb)?;
        let res = Self::parse(unsafe { bh.data() });
        Ok(res)
    }

    fn parse(mut slice: &[u8]) -> Self {
        debug_assert_eq!(slice.len(), BFS_BSIZE as usize);
        let reader = &mut slice;
        Self {
            magic: take_u32(reader),
            start: take_u32(reader),
            end: take_u32(reader),
            from: take_u32(reader),
            to: take_u32(reader),
        }
    }
}

struct BfsInode {
    ino: Ino,
    sblock: u32,
    eblock: u32,
    eoffset: u32,
    vtype: u32,
    mode: u32,
    uid: u32,
    gid: u32,
    nlink: u32,
    atime: u32,
    mtime: u32,
    ctime: u32,
}

impl BfsInode {
    fn parse(mut slice: &[u8]) -> Self {
        debug_assert_eq!(slice.len(), BFS_INODE_SIZE as usize);
        let reader = &mut slice;
        Self {
            ino: {
                let res = Ino(take_u16(reader));
                take_u16(reader); // unused padding
                res
            },
            sblock: take_u32(reader),
            eblock: take_u32(reader),
            eoffset: take_u32(reader),
            vtype: take_u32(reader),
            mode: take_u32(reader),
            uid: take_u32(reader),
            gid: take_u32(reader),
            nlink: take_u32(reader),
            atime: take_u32(reader),
            mtime: take_u32(reader),
            ctime: take_u32(reader),
        }
    }

    fn blocks_taken(&self) -> u32 {
        if self.sblock == 0 {
            0
        } else {
            self.eblock + 1 - self.sblock
        }
    }

    fn file_size(&self) -> u32 {
        if self.sblock == 0 {
            0
        } else {
            self.eoffset + 1 - self.sblock * BFS_BSIZE
        }
    }

    fn serialize(&self, mut slice: &mut [u8]) {
        debug_assert_eq!(slice.len(), BFS_INODE_SIZE as usize);
        let writer = &mut slice;

        write_u16(writer, self.ino.num());
        write_u16(writer, 0);
        write_u32(writer, self.sblock);
        write_u32(writer, self.eblock);
        write_u32(writer, self.eoffset);
        write_u32(writer, self.vtype);
        write_u32(writer, self.mode);
        write_u32(writer, self.uid);
        write_u32(writer, self.gid);
        write_u32(writer, self.nlink);
        write_u32(writer, self.atime);
        write_u32(writer, self.mtime);
        write_u32(writer, self.ctime);
    }

    fn read(sb: &SuperBlock, ino: Ino) -> Result<Self> {
        ino.validate(sb)?;

        let bh = BufferHead::read(sb, ino.block())?;
        let slice = unsafe { bh.data_slice(ino.block_range()) };
        Ok(Self::parse(slice))
    }

    fn write(&self, sb: &SuperBlock) -> Result<BufferHead> {
        let ino = self.ino;
        ino.validate(sb)?;

        let bh = BufferHead::read(sb, ino.block())?;
        let slice = unsafe { bh.data_slice_mut(ino.block_range()) };
        self.serialize(slice);
        unsafe { bindings::mark_buffer_dirty(bh.0) };

        Ok(bh)
    }
}

struct BfsDirent {
    ino: Ino,
    name: [u8; BFS_NAMELEN as usize],
}

impl BfsDirent {
    fn parse(slice: &[u8], offset: u32) -> Self {
        debug_assert_eq!(slice.len(), BFS_BSIZE as usize);
        debug_assert!(offset <= BFS_BSIZE - BFS_DIRENT_SIZE);
        let mut subslice = &slice[offset as usize..][..BFS_DIRENT_SIZE as usize];
        let reader = &mut subslice;
        Self {
            ino: Ino(take_u16(reader)),
            name: (*reader).try_into().unwrap(),
        }
    }

    fn namelen(&self) -> usize {
        self.name.iter().take_while(|&&x| x != b'\0').count()
    }
}

unsafe extern "C" fn bfs_put_super(sb: *mut bindings::super_block) {
    let sb = unsafe { SuperBlock::from_raw(sb) };
    free_sb_info(sb);
}

unsafe extern "C" fn bfs_readdir(f: *mut bindings::file, ctx: *mut bindings::dir_context) -> i32 {
    let dir = unsafe { Inode::from_raw((*f).f_inode) };
    // SAFETY: this function holds exlusive ownership of this reference during its duration
    let ctx = unsafe { &mut *ctx };

    if (ctx.pos & (BFS_DIRENT_SIZE as i64 - 1)) != 0 {
        return code::EINVAL.to_errno();
    }

    let mut iter = DirIterator::new(dir);
    iter.pos = ctx.pos as u32;
    for di in iter
        .by_ref()
        .filter_map(Result::ok)
        .filter(|di| !di.ino.is_unused())
    {
        if unsafe {
            !bindings::dir_emit(
                ctx,
                di.name.as_bytes().as_ptr(),
                di.namelen() as i32,
                di.ino.into(),
                bindings::DT_UNKNOWN,
            )
        } {
            ctx.pos = (iter.pos - BFS_DIRENT_SIZE) as i64;
            return 0;
        }
    }

    ctx.pos = iter.pos as i64;
    0
}

// SAFETY: the returned slice lifetime shouldn't overlap with modification code to qstr
unsafe fn qstr_to_slice_checked<'a>(qstr: *const bindings::qstr, max_len: u32) -> Result<&'a [u8]> {
    unsafe {
        let name_len = (*qstr).__bindgen_anon_1.__bindgen_anon_1.len;
        if name_len > max_len {
            Err(Error::from_errno(-(bindings::ENAMETOOLONG as i32)))
        } else {
            Ok(core::slice::from_raw_parts((*qstr).name, name_len as usize))
        }
    }
}

struct DirIterator<'a> {
    pos: u32,
    size: u32,
    sblock: u32,
    sb: &'a SuperBlock,
    cached_buffer_head: Option<(u32, BufferHead)>,
}

impl<'a> DirIterator<'a> {
    fn new(inode: &'a Inode) -> Self {
        let bfs_inode = inode_to_bfs_inode(inode);

        Self {
            pos: 0,
            size: inode.i_size() as u32,
            sblock: bfs_inode.sblock,
            sb: inode.sb(),
            cached_buffer_head: None,
        }
    }

    fn get_buffer_head(&mut self, block: u32) -> Result<&BufferHead, DiskReadError> {
        let block = self.sblock + block;

        if self.cached_buffer_head.as_ref().map(|(b, _)| *b) != Some(block) {
            let bh = BufferHead::read(self.sb, block).map_err(|_| DiskReadError {})?;
            self.cached_buffer_head = Some((block, bh));
        }

        Ok(&self.cached_buffer_head.as_ref().unwrap().1)
    }
}

struct DiskReadError {}

impl<'a> Iterator for DirIterator<'a> {
    type Item = Result<BfsDirent, DiskReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.size {
            return None;
        }

        let offset = self.pos & (BFS_BSIZE - 1);
        let block = self.pos >> BFS_BSIZE_BITS;

        let block = match self.get_buffer_head(block) {
            Ok(b) => b,
            Err(e) => {
                self.pos += BFS_BSIZE - offset;
                return Some(Err(e));
            }
        };
        let data = unsafe { block.data() };

        let de = BfsDirent::parse(data, offset);
        self.pos += BFS_DIRENT_SIZE;

        Some(Ok(de))
    }
}

unsafe extern "C" fn bfs_lookup(
    dir: *mut bindings::inode,
    dentry: *mut bindings::dentry,
    _flags: u32,
) -> *mut bindings::dentry {
    let name = match unsafe {
        qstr_to_slice_checked(
            core::ptr::addr_of!((*dentry).__bindgen_anon_1.d_name),
            BFS_NAMELEN,
        )
    } {
        Ok(name) => name,
        Err(e) => return e.to_ptr(),
    };

    let dir = unsafe { Inode::from_raw(dir) };

    let found_ino = DirIterator::new(dir)
        .filter_map(Result::ok)
        .find(|de| !de.ino.is_unused() && name == &de.name[..de.namelen()])
        .map(|de| de.ino);

    let inode = match found_ino {
        None => core::ptr::null_mut(),
        Some(ino) => match bfs_iget(dir.sb(), ino) {
            Ok(inode) => inode,
            Err(e) => e.to_ptr(),
        },
    };

    unsafe { bindings::d_splice_alias(inode, dentry) }
}

unsafe extern "C" fn bfs_alloc_inode(_sb: *mut bindings::super_block) -> *mut bindings::inode {
    let inode_info = KBox::new(
        BfsInodeInfo {
            dsk_ino: Ino::UNUSED,
            sblock: 0,
            eblock: 0,
            vfs_inode: pin_init::zeroed(),
        },
        GFP_KERNEL,
    );

    match inode_info {
        Ok(inode_info) => {
            // mmb_init(&bi->i_metadata_bhs, &bi->vfs_inode.i_data);
            let inode = inode_info.vfs_inode.as_raw();

            unsafe {
                bindings::inode_init_once(inode);
            }
            Box::into_raw(inode_info);
            inode
        }
        Err(_) => core::ptr::null_mut(),
    }
}

fn inode_to_bfs_inode(inode: &Inode) -> &BfsInodeInfo {
    unsafe { &*kernel::container_of!(core::ptr::from_ref(inode), BfsInodeInfo, vfs_inode) }
}

// SAFETY: requires exclusive ownership of BfsInodeInfo reference for the duration of its lifetime
unsafe fn inode_to_bfs_inode_mut(inode: &Inode) -> &mut BfsInodeInfo {
    unsafe {
        &mut *kernel::container_of!(
            core::ptr::from_ref(inode).cast_mut(),
            BfsInodeInfo,
            vfs_inode
        )
    }
}

unsafe extern "C" fn bfs_free_inode(inode: *mut bindings::inode) {
    let inode = unsafe { Inode::from_raw(inode) };
    unsafe {
        KBox::<BfsInodeInfo>::from_raw(kernel::container_of!(
            core::ptr::from_ref(inode).cast_mut(),
            BfsInodeInfo,
            vfs_inode
        ))
    };
}

unsafe extern "C" fn bfs_write_inode(
    inode: *mut bindings::inode,
    wbc: *mut bindings::writeback_control,
) -> i32 {
    let inode = unsafe { Inode::from_raw(inode) };
    from_result(|| _bfs_write_inode(inode, wbc))
}

fn _bfs_write_inode(inode: &Inode, wbc: *mut bindings::writeback_control) -> Result<i32> {
    let ino = Ino::from_inode(inode);
    let bi = inode_to_bfs_inode(inode);

    let sb_info: &BfsSbInfo = unsafe { inode.sb().s_fs_info() };
    let lock = sb_info.bfs_lock.lock();

    let bfs_inode = BfsInode {
        ino,
        sblock: bi.sblock,
        eblock: bi.eblock,
        eoffset: if bi.sblock != 0 {
            bi.sblock * BFS_BSIZE + inode.i_size() as u32 - 1
        } else {
            u32::MAX // empty file
        },
        vtype: if ino == Ino::ROOT { BFS_VDIR } else { BFS_VREG },
        mode: inode.i_mode() as u32,
        uid: inode.uid(),
        gid: inode.gid(),
        nlink: inode.nlink(),
        atime: inode.atime_sec() as u32,
        mtime: inode.mtime_sec() as u32,
        ctime: inode.ctime_sec() as u32,
    };
    let bh = bfs_inode.write(inode.sb())?;

    if unsafe { (*wbc).sync_mode } == bindings::writeback_sync_modes_WB_SYNC_ALL {
        unsafe { bindings::sync_dirty_buffer(bh.0) };
        if unsafe { bindings::buffer_req(bh.0) != 0 && bindings::buffer_uptodate(bh.0) == 0 } {
            return Err(code::EIO);
        }
    }

    drop(lock);
    Ok(0)
}

unsafe extern "C" fn bfs_get_block(
    inode: *mut bindings::inode,
    block: u64,
    bh_result: *mut bindings::buffer_head,
    create: i32,
) -> i32 {
    let inode = unsafe { Inode::from_raw(inode) };
    let sb = inode.sb();
    let bi = inode_to_bfs_inode(inode);
    let phys = block.saturating_add(u64::from(bi.sblock));
    if create == 0 {
        if bi.sblock != 0 && phys <= u64::from(bi.eblock) {
            unsafe {
                bindings::map_bh(bh_result, sb.as_raw(), phys);
            }
        }
        return 0;
    }

    /*
     * If the file is not empty and the requested block is within the
     * range of blocks allocated for this file, we can grant it.
     */
    if bi.sblock > 0 && (phys <= u64::from(bi.eblock)) {
        unsafe { bindings::map_bh(bh_result, sb.as_raw(), phys) };
        return 0;
    }

    let info: &BfsSbInfo = unsafe { sb.s_fs_info() };
    // 	/* The file will be extended, so let's see if there is enough space. */
    if phys >= u64::from(info.total_blocks) {
        return code::ENOSPC.to_errno();
    }

    code::ENOSPC.to_errno()

    // 	/* The rest has to be protected against itself. */
    // 	mutex_lock(&info->bfs_lock);

    // 	/*
    // 	 * If the last data block for this file is the last allocated
    // 	 * block, we can extend the file trivially, without moving it
    // 	 * anywhere.
    // 	 */
    // 	if (bi->i_eblock == info->si_lf_eblk) {
    // 		dprintf("c=%d, b=%08lx, phys=%08lx (simple extension)\n",
    // 				create, (unsigned long)block, phys);
    // 		map_bh(bh_result, sb, phys);
    // 		info->si_freeb -= phys - bi->i_eblock;
    // 		info->si_lf_eblk = bi->i_eblock = phys;
    // 		mark_inode_dirty(inode);
    // 		err = 0;
    // 		goto out;
    // 	}

    // 	/* Ok, we have to move this entire file to the next free block. */
    // 	phys = info->si_lf_eblk + 1;
    // 	if (phys + block >= info->si_blocks) {
    // 		err = -ENOSPC;
    // 		goto out;
    // 	}

    // 	if (bi->i_sblock) {
    // 		err = bfs_move_blocks(inode->i_sb, bi->i_sblock,
    // 						bi->i_eblock, phys);
    // 		if (err) {
    // 			dprintf("failed to move ino=%08lx -> fs corruption\n",
    // 								inode->i_ino);
    // 			goto out;
    // 		}
    // 	} else
    // 		err = 0;

    // 	dprintf("c=%d, b=%08lx, phys=%08lx (moved)\n",
    //                 create, (unsigned long)block, phys);
    // 	bi->i_sblock = phys;
    // 	phys += block;
    // 	info->si_lf_eblk = bi->i_eblock = phys;

    // 	/*
    // 	 * This assumes nothing can write the inode back while we are here
    // 	 * and thus update inode->i_blocks! (XXX)
    // 	 */
    // 	info->si_freeb -= bi->i_eblock - bi->i_sblock + 1 - inode->i_blocks;
    // 	mark_inode_dirty(inode);
    // 	map_bh(bh_result, sb, phys);
    // out:
    // 	mutex_unlock(&info->bfs_lock);
    // 	return err;
}

unsafe extern "C" fn bfs_read_folio(
    _file: *mut bindings::file,
    folio: *mut bindings::folio,
) -> i32 {
    unsafe { bindings::block_read_full_folio(folio, Some(bfs_get_block)) }
}

fn bfs_write_failed(mapping: *mut bindings::address_space, to: i64) {
    let inode = unsafe { (*mapping).host };
    let isize = unsafe { Inode::from_raw(inode).i_size() };

    if to > isize {
        unsafe { bindings::truncate_pagecache(inode, isize) };
    }
}

unsafe extern "C" fn bfs_write_begin(
    _iocb: *const bindings::kiocb,
    mapping: *mut bindings::address_space,
    pos: i64,
    len: u32,
    foliop: *mut *mut bindings::folio,
    _fsdata: *mut *mut core::ffi::c_void,
) -> i32 {
    let ret =
        unsafe { bindings::block_write_begin(mapping, pos, len, foliop, Some(bfs_get_block)) };
    if ret != 0 {
        bfs_write_failed(mapping, pos + len as i64);
    }

    ret
}

unsafe extern "C" fn bfs_writepages(
    mapping: *mut bindings::address_space,
    wbc: *mut bindings::writeback_control,
) -> i32 {
    unsafe { bindings::mpage_writepages(mapping, wbc, Some(bfs_get_block)) }
}

unsafe extern "C" fn bfs_statfs(dentry: *mut bindings::dentry, buf: *mut bindings::kstatfs) -> i32 {
    let sb = unsafe { SuperBlock::from_raw((*dentry).d_sb) };
    let info: &BfsSbInfo = unsafe { sb.s_fs_info() };
    let buf = unsafe { KStatFs::from_raw(buf) };
    let id = unsafe { sb.bdev_huge_dev() };

    buf.set_f_type(BFS_MAGIC as isize);
    buf.set_f_bsize(sb.s_blocksize() as isize);
    buf.set_f_blocks(info.total_blocks as u64);
    buf.set_f_bfree(info.free_blocks as u64);
    buf.set_f_bavail(info.free_blocks as u64);
    buf.set_f_files(u64::from(info.last_inode) + 1 - u64::from(Ino::ROOT));
    buf.set_f_ffree(info.free_inodes as u64);
    buf.set_f_fsid(id);
    buf.set_f_namelen(BFS_NAMELEN as isize);

    0
}

const BFS_SOPS: bindings::super_operations = bindings::super_operations {
    alloc_inode: Some(bfs_alloc_inode),
    free_inode: Some(bfs_free_inode),
    write_inode: Some(bfs_write_inode),
    // .evict_inode	= bfs_evict_inode,
    put_super: Some(bfs_put_super),
    statfs: Some(bfs_statfs),
    ..pin_init::zeroed()
};

const BFS_DIR_INOPS: bindings::inode_operations = bindings::inode_operations {
    // .create			= bfs_create,
    lookup: Some(bfs_lookup),
    // .link			= bfs_link,
    // .unlink			= bfs_unlink,
    // .rename			= bfs_rename,
    ..pin_init::zeroed()
};

const BFS_DIR_OPERATIONS: bindings::file_operations = bindings::file_operations {
    read: Some(bindings::generic_read_dir),
    iterate_shared: Some(bfs_readdir),
    // .fsync		= bfs_fsync,
    llseek: Some(bindings::generic_file_llseek),
    ..pin_init::zeroed()
};

const BFS_FILE_INOPS: bindings::inode_operations = bindings::inode_operations {
    ..pin_init::zeroed()
};

const BFS_FILE_OPERATIONS: bindings::file_operations = bindings::file_operations {
    llseek: Some(bindings::generic_file_llseek),
    read_iter: Some(bindings::generic_file_read_iter),
    write_iter: Some(bindings::generic_file_write_iter),
    mmap_prepare: Some(bindings::generic_file_mmap_prepare),
    splice_read: Some(bindings::filemap_splice_read),
    ..pin_init::zeroed()
};

const BFS_AOPS: bindings::address_space_operations = bindings::address_space_operations {
    dirty_folio: Some(bindings::block_dirty_folio),
    invalidate_folio: Some(bindings::block_invalidate_folio),
    read_folio: Some(bfs_read_folio),
    writepages: Some(bfs_writepages),
    write_begin: Some(bfs_write_begin),
    write_end: Some(bindings::generic_write_end),
    // .migrate_folio	= buffer_migrate_folio,
    // .bmap		= bfs_bmap,
    ..pin_init::zeroed()
};

struct LockedInode<'a> {
    inode: &'a Inode,
    locked: bool,
}

impl<'a> LockedInode<'a> {
    unsafe fn get(sb: &SuperBlock, ino: Ino) -> Result<Self> {
        let inode = unsafe { bindings::iget_locked(sb.as_raw(), ino.into()) };
        if inode.is_null() {
            return Err(code::ENOMEM);
        }
        let locked = unsafe { bindings::inode_state_read_once(inode) }
            & bindings::inode_state_flags_enum_I_NEW
            != 0;

        Ok(Self {
            inode: unsafe { Inode::from_raw(inode) },
            locked,
        })
    }

    fn take(self) -> *mut bindings::inode {
        if self.locked {
            unsafe { bindings::unlock_new_inode(self.as_raw()) };
        }

        let ptr = self.inode;
        core::mem::forget(self);
        ptr.as_raw()
    }
}

impl<'a> Deref for LockedInode<'a> {
    type Target = Inode;

    fn deref(&self) -> &Self::Target {
        &self.inode
    }
}

impl<'a> Drop for LockedInode<'a> {
    fn drop(&mut self) {
        if self.locked {
            unsafe { bindings::iget_failed(self.as_raw()) };
        }
    }
}

fn bfs_iget(sb: &SuperBlock, ino: Ino) -> Result<*mut bindings::inode> {
    let inode = unsafe { LockedInode::get(sb, ino)? };
    if !inode.locked {
        return Ok(inode.take());
    }

    let di = BfsInode::read(sb, ino)?;
    inode.set_i_mode(0x00000FFF & di.mode as u16);
    if di.vtype == BFS_VDIR {
        inode.set_i_mode(inode.i_mode() | bindings::S_IFDIR as u16);
        inode.set_i_op(&BFS_DIR_INOPS);
        inode.set_i_fop(&BFS_DIR_OPERATIONS);
    } else if di.vtype == BFS_VREG {
        inode.set_i_mode(inode.i_mode() | bindings::S_IFREG as u16);
        inode.set_i_op(&BFS_FILE_INOPS);
        inode.set_i_fop(&BFS_FILE_OPERATIONS);
        inode.set_mapping_ops(&BFS_AOPS);
    } else {
        return Err(code::EIO);
    };

    {
        let bfs_inode = unsafe { inode_to_bfs_inode_mut(&inode) };
        bfs_inode.sblock = di.sblock;
        bfs_inode.eblock = di.eblock;
        bfs_inode.dsk_ino = di.ino;
    }

    inode.set_uid(di.uid);
    inode.set_gid(di.gid);
    inode.set_nlink(di.nlink);
    inode.set_i_size(di.file_size() as i64);
    inode.set_i_blocks(di.blocks_taken() as u64);
    inode.set_atime(di.atime as i64, 0);
    inode.set_mtime(di.mtime as i64, 0);
    inode.set_ctime(di.ctime as i64, 0);

    Ok(inode.take())
}

fn free_sb_info(sb: &SuperBlock) {
    let info: *mut c_void = unsafe { (*sb.as_raw()).s_fs_info };
    if info.is_null() {
        return;
    }

    unsafe {
        (*sb.as_raw()).s_fs_info = core::ptr::null_mut() as *mut c_void;
        let _info = <Pin<KBox<BfsSbInfo>> as ForeignOwnable>::from_foreign(info);
    }
}

fn _bfs_fill_super(sb: &SuperBlock, _fc: *mut bindings::fs_context) -> Result<i32> {
    unsafe {
        if bindings::sb_set_blocksize(sb.as_raw(), BFS_BSIZE as i32) == 0 {
            return Err(code::EINVAL);
        }
    }

    let sb_header = BfsSuperBlock::read(sb)?;
    if sb_header.magic != BFS_MAGIC {
        return Err(code::EINVAL);
    }

    if sb_header.start > sb_header.end || sb_header.start < BFS_BSIZE + BFS_DIRENT_SIZE {
        return Err(code::EINVAL);
    }

    let last_inode = (sb_header.start - BFS_BSIZE) / BFS_INODE_SIZE + Ino::ROOT.num() as u32 - 1;
    let last_inode = Ino(u16::try_from(last_inode).map_err(|_| code::EINVAL)?);
    if last_inode > Ino::MAX {
        return Err(code::EINVAL);
    }
    let byte_after_end = sb_header.end.checked_add(1).ok_or(code::EINVAL)?;
    let total_blocks = byte_after_end >> BFS_BSIZE_BITS;
    let mut free_blocks = (byte_after_end - sb_header.start) >> BFS_BSIZE_BITS;
    let mut free_inodes = 0;
    let mut inodes_bitmap = BitmapVec::new(usize::from(Ino::MAX) + 1, GFP_KERNEL)?;
    let mut end_block = 0;

    BufferHead::read(sb, total_blocks - 1)?;

    let mut stored_inode_bh = None;
    for ino in (Ino::ROOT.num()..=last_inode.num()).map(Ino) {
        let ino_range = ino.block_range();
        if stored_inode_bh.is_none() || ino_range.start == 0 {
            let bh = BufferHead::read(sb, ino.block())?;
            stored_inode_bh = Some(bh);
        };
        let bh = stored_inode_bh.as_ref().unwrap();

        let data = unsafe { bh.data_slice(ino_range) };
        let inode = BfsInode::parse(data);

        if inode.eblock > total_blocks
            || inode.sblock > inode.eblock
            || (inode.eoffset != u32::MAX && inode.eoffset > sb_header.end)
            || inode.sblock * BFS_BSIZE > inode.eoffset
        {
            return Err(code::EIO);
        }

        if inode.ino.is_unused() {
            free_inodes += 1;
            continue;
        }

        inodes_bitmap.set_bit(ino.into());
        free_blocks = free_blocks.checked_sub(inode.blocks_taken()).ok_or(code::EIO)?;
        end_block = end_block.max(inode.eblock);
    }

    let info = KBox::pin_init(
        try_pin_init!(BfsSbInfo {
            total_blocks,
            free_blocks,
            free_inodes,
            end_block,
            last_inode,
            inodes_bitmap,
            bfs_lock <- new_mutex!(())
        }),
        GFP_KERNEL,
    )?;

    unsafe { sb.set_s_fs_info(info) };
    sb.set_s_op(&BFS_SOPS);

    let inode = bfs_iget(sb, Ino::ROOT).inspect_err(|_| free_sb_info(sb))?;
    let root_dentry = unsafe { bindings::d_make_root(inode) };
    if root_dentry.is_null() {
        free_sb_info(sb);
        return Err(code::ENOMEM);
    }

    sb.set_s_magic(BFS_MAGIC as usize);
    sb.set_s_root(root_dentry);
    sb.set_s_time_min(0);
    sb.set_s_time_max(u32::MAX as i64);

    Ok(0)
}

extern "C" fn bfs_fill_super(sb: *mut bindings::super_block, fc: *mut bindings::fs_context) -> i32 {
    let sb = unsafe { SuperBlock::from_raw(sb) };
    from_result(|| _bfs_fill_super(sb, fc))
}

extern "C" fn bfs_get_tree(fc: *mut bindings::fs_context) -> i32 {
    unsafe { bindings::get_tree_bdev(fc, Some(bfs_fill_super)) }
}

const BFS_CONTEXT_OPS: bindings::fs_context_operations = bindings::fs_context_operations {
    get_tree: Some(bfs_get_tree),
    ..pin_init::zeroed()
};

extern "C" fn bfs_init_fs_context(fc: *mut bindings::fs_context) -> i32 {
    unsafe { (*fc).ops = &BFS_CONTEXT_OPS };

    0
}

impl InPlaceModule for RustBfs {
    fn init(_module: &'static ThisModule) -> impl pin_init::PinInit<Self, kernel::error::Error> {
        pr_info!("Rust bfs (init)\n");
        let mut fst: bindings::file_system_type = Default::default();
        fst.name = c"rust_bfs".as_char_ptr();
        fst.init_fs_context = Some(bfs_init_fs_context);
        fst.kill_sb = Some(bindings::kill_block_super);
        fst.fs_flags = bindings::FS_REQUIRES_DEV as i32;

        try_pin_init!(Self {
            fst <- Opaque::try_ffi_init(move |slot: *mut bindings::file_system_type| {
                // SAFETY: The initializer can write to the provided `slot`.
                unsafe { slot.write(fst) };

                to_result(unsafe { bindings::register_filesystem(slot) })
            }),
        })
    }
}

#[pinned_drop]
impl PinnedDrop for RustBfs {
    fn drop(self: Pin<&mut Self>) {
        pr_info!("Rust bfs (exit)\n");

        unsafe { bindings::unregister_filesystem(self.fst.get()) };
    }
}
