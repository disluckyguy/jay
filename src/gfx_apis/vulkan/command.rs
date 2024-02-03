use {
    crate::gfx_apis::vulkan::{device::VulkanDevice, VulkanError},
    ash::vk::{
        CommandBuffer, CommandBufferAllocateInfo, CommandBufferLevel, CommandPool,
        CommandPoolCreateFlags, CommandPoolCreateInfo,
    },
    std::rc::Rc,
};

pub struct VulkanCommandPool {
    pub(super) device: Rc<VulkanDevice>,
    pub(super) pool: CommandPool,
}

pub struct VulkanCommandBuffer {
    pub(super) pool: Rc<VulkanCommandPool>,
    pub(super) buffer: CommandBuffer,
}

impl Drop for VulkanCommandPool {
    fn drop(&mut self) {
        unsafe {
            self.device.device.destroy_command_pool(self.pool, None);
        }
    }
}

impl Drop for VulkanCommandBuffer {
    fn drop(&mut self) {
        unsafe {
            self.pool
                .device
                .device
                .free_command_buffers(self.pool.pool, &[self.buffer]);
        }
    }
}

impl VulkanCommandPool {
    pub fn allocate_buffer(self: &Rc<Self>) -> Result<Rc<VulkanCommandBuffer>, VulkanError> {
        let create_info = CommandBufferAllocateInfo::builder()
            .command_pool(self.pool)
            .command_buffer_count(1)
            .level(CommandBufferLevel::PRIMARY);
        let buffer = unsafe { self.device.device.allocate_command_buffers(&create_info) };
        let mut buffer = buffer.map_err(VulkanError::AllocateCommandBuffer)?;
        assert_eq!(buffer.len(), 1);
        Ok(Rc::new(VulkanCommandBuffer {
            pool: self.clone(),
            buffer: buffer.pop().unwrap(),
        }))
    }
}

impl VulkanDevice {
    pub fn create_command_pool(self: &Rc<Self>) -> Result<Rc<VulkanCommandPool>, VulkanError> {
        let info = CommandPoolCreateInfo::builder()
            .queue_family_index(self.graphics_queue_idx)
            .flags(
                CommandPoolCreateFlags::TRANSIENT | CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
            );
        let pool = unsafe { self.device.create_command_pool(&info, None) };
        let pool = pool.map_err(VulkanError::AllocateCommandPool)?;
        Ok(Rc::new(VulkanCommandPool {
            device: self.clone(),
            pool,
        }))
    }
}
