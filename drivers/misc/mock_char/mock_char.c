#include <linux/module.h>
#include <linux/fs.h>
#include <linux/types.h>
#include <linux/cdev.h>
#include <linux/uaccess.h>
#include <linux/mutex.h>
#include <linux/slab.h>
#include <linux/list.h>

#define DATA_SIZE 256
#define MAX_MOCK_FILES 256

static int num_devices = 5;

static dev_t dev_num_start;
static struct cdev cdev;

typedef struct {
	struct device *device;
	char data[DATA_SIZE];
	struct mutex data_mutex;
	struct list_head node;
	struct kref ref;
} mock_file_t;

static LIST_HEAD(file_list);
static DEFINE_MUTEX(file_list_mutex);

static void clean_mock_file(struct kref* ref) {
	mock_file_t* mock_file = container_of(ref, mock_file_t, ref);
	mutex_destroy(&mock_file->data_mutex);
	kfree(mock_file);
}

static int mock_open(struct inode *inode, struct file *filp)
{
	struct list_head *cur;
	mock_file_t* mock_file = NULL;
	int minor;
	int i = 0;

	minor = iminor(inode) - MINOR(dev_num_start);
	if (minor >= num_devices) {
		return -EFAULT;
	}

	if (mutex_lock_interruptible(&file_list_mutex)) {
		return -ERESTARTSYS;
	}
	list_for_each(cur, &file_list) {
		if (i == minor) {
			mock_file = list_entry(cur, mock_file_t, node);
			break;
		}
		i++;
	}
	if (mock_file) {
		kref_get(&mock_file->ref);
		filp->private_data = mock_file;
	} else {
		mutex_unlock(&file_list_mutex);
		return -ENODEV;
	}
	mutex_unlock(&file_list_mutex);

	

	pr_info("File open\n");
	return 0;
}

static int mock_release(struct inode *inode, struct file *filp)
{
	mock_file_t* mock_file = filp->private_data;
	kref_put(&mock_file->ref, &clean_mock_file);
	filp->private_data = NULL;
	pr_info("File release\n");
	return 0;
}

static ssize_t mock_read(struct file *filp, char __user *buf, size_t size,
			 loff_t *f_pos)
{
	ssize_t ret;
	size_t transfer_size;
	mock_file_t *mock_file = filp->private_data;

	if (mutex_lock_interruptible(&mock_file->data_mutex)) {
		return -ERESTARTSYS;
	}

	if (*f_pos >= DATA_SIZE) {
		pr_info("Read from position %lld, longer than %d bytes available\n",
			*f_pos, DATA_SIZE);
		ret = 0;
		goto exit;
	}

	transfer_size = min_t(size_t, size, DATA_SIZE - *f_pos);

	if (copy_to_user(buf, mock_file->data + *f_pos, transfer_size)) {
		pr_err("Failed to send data to user!\n");
		ret = -EFAULT;
		goto exit;
	}
	*f_pos = *f_pos + transfer_size;
	pr_info("Read %zd bytes\n", transfer_size);
	ret = transfer_size;

exit:
	mutex_unlock(&mock_file->data_mutex);
	return ret;
}

static ssize_t mock_write(struct file *filp, const char __user *buf,
			  size_t size, loff_t *f_pos)
{
	ssize_t ret;
	size_t transfer_size;
	mock_file_t *mock_file = filp->private_data;

	if (mutex_lock_interruptible(&mock_file->data_mutex)) {
		return -ERESTARTSYS;
	}

	if (*f_pos >= DATA_SIZE) {
		ret = 0;
		goto exit;
	}

	transfer_size = min_t(size_t, size, DATA_SIZE - *f_pos);

	if (copy_from_user(mock_file->data + *f_pos, buf, transfer_size)) {
		pr_err("Failed to send data from user!\n");
		ret = -EFAULT;
		goto exit;
	}
	*f_pos = *f_pos + transfer_size;

	pr_info("Written %zd bytes\n", transfer_size);
	ret = transfer_size;

exit:
	mutex_unlock(&mock_file->data_mutex);
	return ret;
}

static loff_t mock_llseek(struct file *filp, loff_t offset, int whence)
{
	loff_t ret;
	mock_file_t *mock_file = filp->private_data;

	if (mutex_lock_interruptible(&mock_file->data_mutex)) {
		return -ERESTARTSYS;
	}

	if (whence == SEEK_SET) {
		ret = offset;
	} else if (whence == SEEK_CUR) {
		ret = filp->f_pos + offset;
	} else if (whence == SEEK_END) {
		ret = DATA_SIZE + offset;
	} else {
		return -EINVAL;
	}

	if (ret < 0 || ret > DATA_SIZE) {
		return -EINVAL;
	}

	filp->f_pos = ret;

	return ret;
}

static const struct file_operations mock_fops = {
	.owner = THIS_MODULE,
	.open = mock_open,
	.release = mock_release,
	.read = mock_read,
	.write = mock_write,
	.llseek = mock_llseek,
};

static const struct class mock_class = {
	.name = "mockchar",
};

static void clean_mock_list(void)
{
	mock_file_t *entry, *tmp;
	int i = 0;

	mutex_lock(&file_list_mutex);
	list_for_each_entry_safe(entry, tmp, &file_list, node) {
		device_destroy(&mock_class, dev_num_start + i);

		list_del(&entry->node);
		kref_put(&entry->ref, clean_mock_file);
		i++;
	}
	mutex_unlock(&file_list_mutex);
}

static int create_new_mock_file(int minor) {
	struct device *device;
	mock_file_t *file;
	int ret;

	file = kzalloc(sizeof(mock_file_t), GFP_KERNEL);
	if (!file) {
		pr_err("Failed to allocate memory for file\n");
		return -ENOMEM;
	}

	mutex_init(&file->data_mutex);

	device = device_create(&mock_class, NULL,
					dev_num_start + minor, NULL, "%s%d",
					"mockchar", minor);

	if (IS_ERR(device)) {
		kfree(file);
		ret = PTR_ERR(device);
		pr_err("Failed to create device (err %d)\n", -ret);
		return ret;
	}

	kref_init(&file->ref);

	file->device = device;
	list_add_tail(&file->node, &file_list);
	
	return 0;
}

static int set_num_devices_param(const char *val, const struct kernel_param *kp)
{
	int n, ret, difference, i;
	mock_file_t* mock_file;

	ret = kstrtoint(val, 10, &n);
	if (ret != 0 || n < 1 || n > MAX_MOCK_FILES)
		return -EINVAL;

	difference = n - num_devices;

	if (mutex_lock_interruptible(&file_list_mutex)) {
		return -ERESTARTSYS;
	}
	if (difference > 0) {
		for (i = 0; i < difference; i++) {
			ret = create_new_mock_file(num_devices);
			if (ret) {
				mutex_unlock(&file_list_mutex);
				return ret;
			} else {
				num_devices++;
			}
		}
	} else if (difference < 0) {
		for (i = 0; i < -difference; i++) {
			mock_file = list_last_entry(&file_list, mock_file_t, node);
			device_destroy(&mock_class, dev_num_start + num_devices - 1);

			list_del(&mock_file->node);
			kref_put(&mock_file->ref, clean_mock_file);
			num_devices--;
		}
	}
	mutex_unlock(&file_list_mutex);


	pr_info("Set num_devices parameter to %d\n", num_devices);

	return 0;
}

static const struct kernel_param_ops num_devices_param_ops = {
	.set = set_num_devices_param,
	.get = param_get_int,
};

module_param_cb(num_devices, &num_devices_param_ops, &num_devices,
		S_IRUSR | S_IWUSR);
MODULE_PARM_DESC(num_devices, "Number of mock devices to create");

static int mock_init(void)
{
	int ret, i;

	ret = alloc_chrdev_region(&dev_num_start, 0, MAX_MOCK_FILES,
				  "mockchar");
	if (ret) {
		pr_err("Failed to allocate character device region (err %d)!",
		       -ret);
		goto exit1;
	}

	cdev_init(&cdev, &mock_fops);
	cdev.owner = THIS_MODULE;
	ret = cdev_add(&cdev, dev_num_start, MAX_MOCK_FILES);
	if (ret) {
		pr_err("Failed to add a kernel device (err %d)!\n", -ret);
		goto exit2;
	}

	ret = class_register(&mock_class);
	if (ret) {
		pr_err("Failed to register class (err %d)!\n", -ret);
		goto exit3;
	}

	if (mutex_lock_interruptible(&file_list_mutex)) {
		ret = -ERESTARTSYS;
		goto exit4;
	}
	for (i = 0; i < num_devices; i++) {
		ret = create_new_mock_file(i);
		if (ret) {
			mutex_unlock(&file_list_mutex);
			goto exit4;
		}
	}
	mutex_unlock(&file_list_mutex);

	pr_info("Initialized successfully\n");
	return 0;
exit4:
	clean_mock_list();
	class_unregister(&mock_class);
exit3:
	cdev_del(&cdev);
exit2:
	unregister_chrdev_region(dev_num_start, MAX_MOCK_FILES);
exit1:
	return ret;
}

static void mock_delete(void)
{
	cdev_del(&cdev);
	clean_mock_list();
	class_unregister(&mock_class);
	unregister_chrdev_region(dev_num_start, MAX_MOCK_FILES);
	pr_info("Removed successfully\n");
}

module_init(mock_init);
module_exit(mock_delete);

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Nothing here");