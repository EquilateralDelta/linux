#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/netfilter.h>
#include <linux/netfilter_ipv4.h>
#include <linux/skbuff.h>
#include <linux/ip.h>

static struct nf_hook_ops nfho;

static unsigned int hook_func(void *priv, struct sk_buff *skb,
			      const struct nf_hook_state *state)
{
	struct iphdr *ip_header;
	struct icmphdr *icmp_header;

	if (!skb)
		return NF_ACCEPT;

	ip_header = ip_hdr(skb); // Get IP header

	if (ip_header->protocol == IPPROTO_ICMP) {
		// iph->ihl is the number of 32-bit words. Multiply by 4 to get bytes.
		icmp_header = (struct icmphdr *)((char *)ip_header + (ip_header->ihl * 4));
	}

	// Example: Print packet length and pointer address
	pr_info("SKB Inspect: len=%u, head=%p, data=%p\n", skb->len, skb->head,
		skb->data);

	return NF_ACCEPT; // Pass packet through
}

static int __init skb_mod_init(void)
{
	nfho.hook = hook_func;
	nfho.hooknum = NF_INET_POST_ROUTING; // Catch packets as they arrive
	nfho.pf = PF_INET;
	nfho.priority = NF_IP_PRI_FIRST;
	nf_register_net_hook(&init_net, &nfho);
	return 0;
}

static void __exit skb_mod_exit(void)
{
	nf_unregister_net_hook(&init_net, &nfho);
}

module_init(skb_mod_init);
module_exit(skb_mod_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Test module to inspect sk_buff structures");