#include <linux/mpage.h>

__rust_helper int rust_helper_mpage_writepages(struct address_space *mapping,
		struct writeback_control *wbc, get_block_t get_block)
{
	return mpage_writepages(mapping, wbc, get_block);
}