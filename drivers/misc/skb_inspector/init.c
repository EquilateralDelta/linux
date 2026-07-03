#include <linux/module.h>
#include <linux/printk.h>
#include "ping_handler.h"
#include "net_hooks.h"


static int __init skb_mod_init(void)
{
	int ret;
	pr_info("SKB inspector: Module loaded\n");

	ret = init_hooks();
	if (ret) {
		pr_err("SKB inspector: Failed to init hooks\n");
		goto err1;
	}

	ret = init_ping_handler();
	if (ret) {
		pr_err("SKB inspector: Failed to register ping handler, %d\n", ret);
		goto err2;
	}
	
	return ret;
err2:
	clear_hooks();
err1:
	return ret;
}

static void __exit skb_mod_exit(void)
{
	pr_info("SKB inspector: Module unloaded\n");

	clear_ping_handler();
	clear_hooks();
}

module_init(skb_mod_init);
module_exit(skb_mod_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Test module to inspect sk_buff structures");