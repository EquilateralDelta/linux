// SPDX-License-Identifier: GPL-2.0

/*
 * Copyright (C) 2024 Google LLC.
 */

#include <linux/fs.h>

__rust_helper struct file *rust_helper_get_file(struct file *f)
{
	return get_file(f);
}

__rust_helper enum inode_state_flags_enum rust_helper_inode_state_read_once(struct inode *inode)
{
	return inode_state_read_once(inode);
}

__rust_helper void rust_helper_i_uid_write(struct inode *inode, uid_t uid)
{
	return i_uid_write(inode, uid);
}

__rust_helper void rust_helper_i_gid_write(struct inode *inode, gid_t gid)
{
	return i_gid_write(inode, gid);
}

__rust_helper uid_t rust_helper_i_uid_read(const struct inode *inode)
{
	return i_uid_read(inode);
}

__rust_helper gid_t rust_helper_i_gid_read(const struct inode *inode)
{
	return i_gid_read(inode);
}

__rust_helper time64_t rust_helper_inode_get_atime_sec(const struct inode *inode)
{
	return inode_get_atime_sec(inode);
}

__rust_helper time64_t rust_helper_inode_get_mtime_sec(const struct inode *inode)
{
	return inode_get_mtime_sec(inode);
}

__rust_helper time64_t rust_helper_inode_get_ctime_sec(const struct inode *inode)
{
	return inode_get_ctime_sec(inode);
}

__rust_helper struct timespec64 rust_helper_inode_set_atime(struct inode *inode,
						time64_t sec, long nsec)
{
	return inode_set_atime(inode, sec, nsec);
}

__rust_helper struct timespec64 rust_helper_inode_set_mtime(struct inode *inode,
						time64_t sec, long nsec)
{
	return inode_set_mtime(inode, sec, nsec);
}

__rust_helper struct timespec64 rust_helper_inode_set_ctime(struct inode *inode,
						time64_t sec, long nsec)
{
	return inode_set_ctime(inode, sec, nsec);
}

__rust_helper void rust_helper_mark_inode_dirty(struct inode *inode)
{
	mark_inode_dirty(inode);
}

__rust_helper bool rust_helper_dir_emit(struct dir_context *ctx,
			    const char *name, int namelen,
			    u64 ino, unsigned type)
{
	return dir_emit(ctx, name, namelen, ino, type);
}