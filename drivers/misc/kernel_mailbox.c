#include <linux/module.h>
#include <linux/types.h>
#include <linux/cdev.h>
#include <linux/fs.h>
#include <linux/mutex.h>
#include <linux/string.h>
#include <linux/wait.h>

#define DATA_SIZE 32
#define MAX_FILES 1

static dev_t dev_num;
static struct cdev cdev;
static struct device *device;
static char data[DATA_SIZE];
static int data_head;
static int data_tail;
static struct mutex data_mutex;
DECLARE_WAIT_QUEUE_HEAD(data_read_wq);
DECLARE_WAIT_QUEUE_HEAD(data_write_wq);

static void data_reset(void) {
	memset(data, 0, DATA_SIZE);
	data_head = 0;
	data_tail = 0;
}

static void data_init(void) {
	data_reset();
	mutex_init(&data_mutex);
}

static void data_release(void) {
	data_reset();
	mutex_destroy(&data_mutex);
}

static ssize_t buffer_data_size_continuous(void) {
	if (data_tail >= data_head) {
		return data_tail - data_head;
	}

	return DATA_SIZE - data_head;
}

static ssize_t buffer_free_size_continuous(void) {
	if (data_tail < data_head) {
		return data_head - data_tail - 1;
	}

	return DATA_SIZE - data_tail - ((data_head == 0) ? 1 : 0);
}

static int mailbox_open(struct inode *inode, struct file *filp)
{
	pr_info("Mailbox file open\n");
	return 0;
}

static int mailbox_release(struct inode *inode, struct file *filp)
{
	pr_info("Mailbox file release\n");
	return 0;
}


static ssize_t mailbox_read(struct file *filp, char __user *buf, size_t size,
			 loff_t *f_pos)
{
	ssize_t ret;
	size_t transfer_size;

	if (mutex_lock_interruptible(&data_mutex)) {
		return -ERESTARTSYS;
	}

	while (!buffer_data_size_continuous()) {
		mutex_unlock(&data_mutex);
		if (filp->f_flags & O_NONBLOCK)
			return -EAGAIN;
		
		if (wait_event_interruptible(data_read_wq, buffer_data_size_continuous()))
			return -ERESTARTSYS;

		if (mutex_lock_interruptible(&data_mutex)) {
			return -ERESTARTSYS;
		}
	}

	transfer_size = min_t(size_t, size, buffer_data_size_continuous());

	if (copy_to_user(buf, data, transfer_size)) {
		pr_err("Failed to send data to user!\n");
		ret = -EFAULT;
		goto exit;
	}

	data_head = (data_head + transfer_size) & (DATA_SIZE - 1);
	mutex_unlock(&data_mutex);
	wake_up_interruptible(&data_write_wq);
	return transfer_size;
exit:
	mutex_unlock(&data_mutex);
	return ret;
}

static ssize_t mailbox_write(struct file *filp, const char __user *buf,
			  size_t size, loff_t *f_pos)
{
	ssize_t ret;
	size_t transfer_size;

	if (mutex_lock_interruptible(&data_mutex)) {
		return -ERESTARTSYS;
	}

	while (!buffer_free_size_continuous()) {
		mutex_unlock(&data_mutex);
		if (filp->f_flags & O_NONBLOCK)
			return -EAGAIN;
		
		if (wait_event_interruptible(data_write_wq, buffer_free_size_continuous()))
			return -ERESTARTSYS;

		if (mutex_lock_interruptible(&data_mutex)) {
			return -ERESTARTSYS;
		}
	}

	transfer_size = min_t(size_t, size, buffer_free_size_continuous());
	if (copy_from_user(data, buf, transfer_size)) {
		pr_err("Failed to send data from user!\n");
		ret = -EFAULT;
		goto exit;
	}

	data_tail = (data_tail + transfer_size) & (DATA_SIZE - 1);
	mutex_unlock(&data_mutex);
	wake_up_interruptible(&data_read_wq);
	return transfer_size;
exit:
	mutex_unlock(&data_mutex);
	return ret;
}

static const struct file_operations mailbox_fops = {
	.owner = THIS_MODULE,
	.open = mailbox_open,
	.release = mailbox_release,
	.read = mailbox_read,
	.write = mailbox_write,
};

static const struct class mailbox_class = {
	.name = "mailbox",
};

static int m_init(void)
{
	int ret;

	ret = alloc_chrdev_region(&dev_num, 0, MAX_FILES, "mailbox");
	if (ret) {
		pr_err("Failed to allocate character device region (err %d)!",
		       -ret);
		goto exit1;
	}

	cdev_init(&cdev, &mailbox_fops);
	cdev.owner = THIS_MODULE;
	ret = cdev_add(&cdev, dev_num, MAX_FILES);
	if (ret) {
		pr_err("Failed to add a kernel device (err %d)!\n", -ret);
		goto exit2;
	}

	ret = class_register(&mailbox_class);
	if (ret) {
		pr_err("Failed to register class (err %d)!\n", -ret);
		goto exit3;
	}

	device = device_create(&mailbox_class, NULL, dev_num, NULL, "%s",
			       "mailbox");

	if (IS_ERR(device)) {
		ret = PTR_ERR(device);
		pr_err("Failed to create device (err %d)\n", -ret);
		goto exit4;
	}

	data_init();

	pr_info("Kernel mailbox module loaded\n");
	return 0;

exit4:
	class_unregister(&mailbox_class);
exit3:
	cdev_del(&cdev);
exit2:
	unregister_chrdev_region(dev_num, MAX_FILES);
exit1:
	return ret;
}

static void m_delete(void)
{
	device_destroy(&mailbox_class, dev_num);
	class_unregister(&mailbox_class);
	cdev_del(&cdev);
	unregister_chrdev_region(dev_num, MAX_FILES);
	data_release();

	pr_info("Kernel mailbox module unloaded\n");
}

module_init(m_init);
module_exit(m_delete);

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Nothing here");