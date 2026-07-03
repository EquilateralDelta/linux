#include <linux/netfilter.h>
#include <linux/printk.h>
#include <linux/ip.h>
#include <linux/icmp.h>
#include <linux/netfilter_ipv4.h>
#include "net_hooks.h"

static char log_buffer_storage[512];
static char *log_buffer = log_buffer_storage;

static __printf(3, 4) void format_and_log(char *buf, size_t size,
					  const char *fmt, ...)
{
	va_list args;

	va_start(args, fmt);
	vsnprintf(buf, size, fmt, args);
	va_end(args);

	pr_info("%s\n", buf);
}


static unsigned int __attribute__((optimize("O0")))
hook_func(void *priv, struct sk_buff *skb, const struct nf_hook_state *state)
{
	struct iphdr *ip_header;
	struct icmphdr *icmp_header;
	unsigned char *data;
	bool is_ping_request;

	if (!skb)
		return NF_ACCEPT;

	ip_header = ip_hdr(skb); // Get IP header

	pr_info("SKB inspector: message with protocol %d\n",
				ip_header->protocol);

	if (ip_header->protocol != IPPROTO_ICMP) {
		return NF_ACCEPT;
	}

	icmp_header =
		(struct icmphdr *)((char *)ip_header + (ip_header->ihl * 4));

	if (icmp_header->type == 8 && icmp_header->code == 0) {
		is_ping_request = true;
	} else if (icmp_header->type == 0 && icmp_header->code == 0) {
		is_ping_request = false;
	} else {
		return NF_ACCEPT;
	}

	data = (char *)icmp_header + sizeof(struct icmphdr);

	format_and_log(
		log_buffer, sizeof(log_buffer_storage),
		"SKB inspector: %s %pI4 to %pI4, id: %u, sequence: %u, data: %s",
		is_ping_request ? "Request" : "Response", &ip_header->saddr,
		&ip_header->daddr, ntohs(icmp_header->un.echo.id),
		ntohs(icmp_header->un.echo.sequence), data);

	return NF_ACCEPT;
}

static struct nf_hook_ops inspector_ops[] = {
	{
		.hook = hook_func,
		.pf = NFPROTO_IPV4,
		.hooknum =
			NF_INET_PRE_ROUTING, // Catch incoming (and before routing decisions)
		.priority =
			NF_IP_PRI_FIRST, // Run before iptables/nftables modifiers
	},
	{
		.hook = hook_func,
		.pf = NFPROTO_IPV4,
		.hooknum =
			NF_INET_POST_ROUTING, // Catch outgoing (and after routing decisions)
		.priority =
			NF_IP_PRI_LAST, // Run after final modifications are complete
	}
};

int init_hooks(void)
{
	return nf_register_net_hooks(&init_net, inspector_ops,
				     ARRAY_SIZE(inspector_ops));
}

void clear_hooks(void) 
{
    nf_unregister_net_hooks(&init_net, inspector_ops, ARRAY_SIZE(inspector_ops));
}