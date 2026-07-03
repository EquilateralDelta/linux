#include "linux/workqueue.h"
#include <linux/string.h>
#include <linux/printk.h>
#include <linux/netdevice.h>
#include <linux/etherdevice.h>
#include <linux/ip.h>
#include <linux/icmp.h>
#include <linux/debugfs.h>
#include <linux/net.h>
#include <net/sock.h>
#include "ping_handler.h"

#define PING_ID htons(1234)

static struct dentry *dbg_file;
static struct socket *socket;
static struct work_struct work;

static inline __be32 make_ipv4(u8 a, u8 b, u8 c, u8 d)
{
	return htonl(((u32)a << 24) | ((u32)b << 16) | ((u32)c << 8) | d);
}

static int __attribute__((optimize("O0")))
send_ping_package(struct net_device *dev)
{
	struct sk_buff *skb;
	char *data;
	char *packet_data = "abcdef123456";
	__be32 dst_addr = make_ipv4(10, 0, 2, 2);
	struct icmphdr *icmp_header;
	struct iphdr *ip_header;
	struct ethhdr *eth_header;

	skb = netdev_alloc_skb(dev, dev->mtu);
	if (!skb) {
		return -1;
	}

	skb_reserve(skb, LL_RESERVED_SPACE(dev));

	data = skb_put(skb, strlen(packet_data));
	memcpy(data, packet_data, strlen(packet_data));

	icmp_header = skb_push(skb, sizeof(struct icmphdr));
	skb_reset_transport_header(skb);

	icmp_header->type = 8;
	icmp_header->code = 0;
	icmp_header->un.echo.id = PING_ID;
	icmp_header->un.echo.sequence = htons(5678);
	icmp_header->checksum = 0;
	icmp_header->checksum = ip_compute_csum(icmp_header, skb->len);


	ip_header = skb_push(skb, sizeof(struct iphdr));
	skb_reset_network_header(skb);

	ip_header->version = 4;
	ip_header->ihl = 5;
	ip_header->tos = 0;
	ip_header->tot_len = htons(skb->len);
	ip_header->id = htons(3333);
	ip_header->frag_off = htons(0);
	ip_header->ttl = 128;
	ip_header->protocol = IPPROTO_ICMP;
	ip_header->saddr = make_ipv4(10, 0, 2, 15);
	ip_header->daddr = dst_addr;
	ip_header->check = 0;
	ip_header->check = ip_compute_csum(ip_header, sizeof(struct iphdr));

	eth_header = skb_push(skb, ETH_HLEN);
	skb_reset_mac_header(skb);

	char c[] = { 0x52, 0x55, 0x0a, 0x00, 0x02, 0x02 };
	//52:54:00:12:34:56
	ether_addr_copy(eth_header->h_dest, c);
	ether_addr_copy(eth_header->h_source, dev->dev_addr);

	eth_header->h_proto = htons(ETH_P_IP);
	skb->dev = dev;
	skb->protocol = htons(ETH_P_IP);

	return dev_queue_xmit(skb);
}

static ssize_t send_ping_write(struct file *f, const char __user *buf,
                               size_t len, loff_t *ppos)
{
    char devname[IFNAMSIZ];
    struct net_device *dev;

    if (len >= IFNAMSIZ) return -EINVAL;
    if (copy_from_user(devname, buf, len)) return -EFAULT;
    devname[len] = '\0';
    strim(devname);

    dev = dev_get_by_name(&init_net, devname);
    if (!dev) return -ENODEV;
    send_ping_package(dev);
    dev_put(dev);
    return len;
}

static const struct file_operations send_ping_fops = {
    .write = send_ping_write,
    .owner = THIS_MODULE,
};

static void ping_receiver(struct work_struct *work)
{
	struct sock* sk = socket->sk;
	struct sk_buff* skb;
	struct icmphdr* icmp_header;
	int err;
	while((skb = skb_recv_datagram(sk, MSG_DONTWAIT, &err))) {
		icmp_header = icmp_hdr(skb);
		if (icmp_header->un.echo.id != PING_ID) {
			skb_free_datagram(sk, skb);
			continue;
		}

		pr_info("SKB inspector: received ping response with seq number %d\n", ntohs(icmp_header->un.echo.sequence));

		skb_free_datagram(sk, skb);
	}
}

static void response_ready(struct sock *sk) 
{
	schedule_work(&work);
}

int init_ping_handler(void) {
	int err;

    dbg_file = debugfs_create_file("send_ping", 0200, NULL, NULL, &send_ping_fops);
    if (IS_ERR(dbg_file)) {
        err = -1;
		goto err1;
    }

	err = sock_create_kern(&init_net, PF_INET, SOCK_RAW, IPPROTO_ICMP, &socket);
	if (err) {
		pr_info("SKB inspector: failed to create a socket, error %d\n", err);
		err = -2;
		goto err2;
	}

	INIT_WORK(&work, ping_receiver);

	struct sock* sock = socket->sk;
	write_lock_bh(&sock->sk_callback_lock);
	sock->sk_data_ready = response_ready;
	write_unlock_bh(&sock->sk_callback_lock);

	return 0;

err2:
	debugfs_remove_recursive(dbg_file);
err1:
    return err;
}

void clear_ping_handler(void) {
	cancel_work_sync(&work);
	sock_release(socket);
    debugfs_remove_recursive(dbg_file);
}