// SPDX-License-Identifier: GPL-2.0

#include <linux/buffer_head.h>

__rust_helper struct buffer_head *rust_helper_sb_bread(struct super_block *sb,
                                                         sector_t block)
{
    return sb_bread(sb, block);
}

__rust_helper void rust_helper_brelse(struct buffer_head *bh) 
{
    return brelse(bh);
}

__rust_helper void rust_helper_map_bh(struct buffer_head *bh, struct super_block *sb, sector_t block) 
{
    return map_bh(bh, sb, block);
}