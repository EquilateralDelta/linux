#include "linux/gfp_types.h"
#include "linux/spinlock_types.h"
#include <linux/types.h>
#include <linux/spinlock.h>
#include <linux/list.h>
#include <linux/slab.h>
#include <linux/timekeeping.h>
#include <linux/string.h>
#include <linux/printk.h>
#include <linux/netdevice.h>
#include <linux/etherdevice.h>
#include <linux/ip.h>
#include <linux/icmp.h>
#include <linux/debugfs.h>
#include <linux/net.h>
#include <net/sock.h>
#include <net/route.h>
#include <net/ip.h>
#include "ping_handler.h"

#define PING_ID htons(1234)

static struct dentry *dbg_file;
static struct socket *socket;
static struct work_struct work;
static atomic_t sequence = ATOMIC_INIT(5678);
static DEFINE_SPINLOCK(send_req_lock);
static LIST_HEAD(send_req);
struct ping_req {
	u16 seq;
	ktime_t time;
	struct list_head node;
};

static inline __be32 make_ipv4(u8 a, u8 b, u8 c, u8 d)
{
	return htonl(((u32)a << 24) | ((u32)b << 16) | ((u32)c << 8) | d);
}

static __used noinline int dbg_skb_cloned(struct sk_buff *skb)
{
	return skb_cloned(skb);
}
static __used noinline int dbg_skb_shared(struct sk_buff *skb)
{
	return skb_shared(skb);
}

static struct dst_entry *get_ping_dst(__be32 src_addr, __be32 dst_addr)
{
	struct flowi4 fl = { { 0 } };
	struct rtable *rt;

	memset(&fl, 0, sizeof(fl));
	memcpy(&fl.saddr, &src_addr, sizeof(src_addr));
	memcpy(&fl.daddr, &dst_addr, sizeof(dst_addr));

	rt = ip_route_output_key(&init_net, &fl);
	if (IS_ERR(rt)) {
		pr_info("SKB inspector: failed to send ping because of missing route!\n");
		return NULL;
	}

	return &rt->dst;
}

static int send_ping_package(void)
{
	struct sk_buff *skb;
	char *data;
	char *payload = "abcdef123456";
	struct net_device *dev;
	__be32 src_addr = make_ipv4(10, 0, 2, 15);
	__be32 dst_addr = make_ipv4(10, 0, 2, 2);
	struct icmphdr *icmph;
	int req_seq = atomic_inc_return(&sequence);
	struct dst_entry *dst = get_ping_dst(src_addr, dst_addr);
	struct ping_req *ping_req;

	if (!dst) {
		return -1;
	}
	dev = dst->dev;

	skb = netdev_alloc_skb(dev, dev->mtu);
	if (!skb) {
		goto err_skb;
	}

	ping_req = kmalloc(sizeof(*ping_req), GFP_KERNEL);
	if (!ping_req) {
		goto err_ping_req;
	}
	ping_req->seq = req_seq;
	ping_req->time = ktime_get();
	spin_lock(&send_req_lock);
	list_add(&ping_req->node, &send_req);
	spin_unlock(&send_req_lock);

	skb_dst_set(skb, dst);
	skb_reserve(skb, LL_RESERVED_SPACE(dev));

	data = skb_put(skb, strlen(payload));
	memcpy(data, payload, strlen(payload));

	icmph = skb_push(skb, sizeof(struct icmphdr));
	skb_reset_transport_header(skb);

	icmph->type = 8;
	icmph->code = 0;
	icmph->un.echo.id = PING_ID;
	icmph->un.echo.sequence = htons(req_seq);
	icmph->checksum = 0;
	icmph->checksum = ip_compute_csum(icmph, skb->len);

	return ip_build_and_send_pkt(skb, socket->sk, src_addr, dst_addr, NULL,
				     0);

err_ping_req:
	kfree_skb(skb);
err_skb:
	dst_release(dst);
	return -1;
}

static ssize_t send_ping_write(struct file *f, const char __user *buf,
			       size_t len, loff_t *ppos)
{
	send_ping_package();
	return len;
}

static const struct file_operations send_ping_fops = {
	.write = send_ping_write,
	.owner = THIS_MODULE,
};

static void ping_receiver(struct work_struct *work)
{
	struct sock *sk = socket->sk;
	struct sk_buff *skb;
	struct icmphdr *icmph;
	struct ping_req *req;
	int err;

	while ((skb = skb_recv_datagram(sk, MSG_DONTWAIT, &err))) {
		icmph = icmp_hdr(skb);
		if (icmph->un.echo.id != PING_ID) {
			skb_free_datagram(sk, skb);
			continue;
		}

		u16 res_seq = ntohs(icmph->un.echo.sequence);
		ktime_t req_time = -1;
		spin_lock(&send_req_lock);
		list_for_each_entry(req, &send_req, node) {
			if (req->seq == res_seq) {
				req_time = req->time;
				list_del(&req->node);
				break;
			}
		}
		spin_unlock(&send_req_lock);

		if (req_time != -1) {
			kfree(req);

			pr_info("SKB inspector: received ping response with seq number %d, time elapsed %lld\n",
				res_seq,
				ktime_ms_delta(ktime_get(), req_time));
		} else {
			pr_info("SKB inspector: received ping response with seq number %d, not found send time\n",
				res_seq);
		}

		skb_free_datagram(sk, skb);
	}
}

static void response_ready(struct sock *sk)
{
	schedule_work(&work);
}

int init_ping_handler(void)
{
	int err;

	dbg_file = debugfs_create_file("send_ping", 0200, NULL, NULL,
				       &send_ping_fops);
	if (IS_ERR(dbg_file)) {
		err = -1;
		goto err1;
	}

	err = sock_create_kern(&init_net, PF_INET, SOCK_RAW, IPPROTO_ICMP,
			       &socket);
	if (err) {
		pr_info("SKB inspector: failed to create a socket, error %d\n",
			err);
		err = -2;
		goto err2;
	}

	INIT_WORK(&work, ping_receiver);

	struct sock *sock = socket->sk;
	write_lock_bh(&sock->sk_callback_lock);
	sock->sk_data_ready = response_ready;
	write_unlock_bh(&sock->sk_callback_lock);

	return 0;

err2:
	debugfs_remove_recursive(dbg_file);
err1:
	return err;
}

void clear_ping_handler(void)
{
	cancel_work_sync(&work);
	sock_release(socket);
	debugfs_remove_recursive(dbg_file);
}