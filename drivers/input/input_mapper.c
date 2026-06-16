#include <linux/module.h>
#include <linux/input.h>

static int mapper_connect(struct input_handler *handler, struct input_dev *dev,
			  const struct input_device_id *id)
{
    pr_info("Mapper handler connected for device %d:%d\n", id->vendor, id->product);
	return 0;
}

static void mapper_disconnect(struct input_handle *handle)
{
    pr_info("Mapper handler disconnected\n");
}

static bool mapper_filter(struct input_handle *handle, unsigned int type, unsigned int code, int value) {
    return false;
}

static const struct input_device_id mapper_ids[] = {
	{ },	/* Terminating entry */
};

static struct input_handler mapper_handler = {
    .connect	= mapper_connect,
	.disconnect	= mapper_disconnect,
    .filter = mapper_filter,
    .name = "mapper",
    .id_table = mapper_ids,
};

static int m_init(void)
{
    int ret;
    ret = input_register_handler(&mapper_handler);
    if (ret) {
        pr_err("Input mapper: Failed to register handlers\n");
        return ret;
    }

	pr_info("Input mapper module loaded\n");
	return 0;
}

static void m_delete(void)
{
    input_unregister_handler(&mapper_handler);
	pr_info("Input mapper module unloaded\n");
}

module_init(m_init);
module_exit(m_delete);

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Nothing here");