use super::*;

fn binding_key(vm: &DynamipsVm, adapter: u32, port: u32) -> (String, u32, u32) {
    (vm.node_id.clone(), adapter, port)
}

fn nio_name(vm: &DynamipsVm, adapter: u32, port: u32) -> String {
    format!("udp-{}-{adapter}-{port}", vm.node_id.replace('-', ""))
}

impl DynamipsRuntime {
    pub async fn switch_endpoint(&self, vm: &DynamipsVm, adapter_number: u32, port_number: u32, switch_port: u16) -> Result<u16, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        self.ensure_hypervisor(vm).await?;
        let key = binding_key(vm, adapter_number, port_number);
        if let Some(binding) = self.bindings.lock().await.get(&key) { return Ok(binding.local_port); }
        let local_port = available_udp_port().await?;
        let name = nio_name(vm, adapter_number, port_number);
        self.send(vm.hypervisor_port, &format!("nio create_udp {name} {local_port} 127.0.0.1 {switch_port}")).await?;
        self.send(vm.hypervisor_port, &format!("vm slot_add_nio_binding {} {adapter_number} {port_number} {name}", quote(&vm.name))).await?;
        if self.hypervisor_status(vm.hypervisor_port, &vm.name).await? == "started" {
            self.send(vm.hypervisor_port, &format!("vm slot_enable_nio {} {adapter_number} {port_number}", quote(&vm.name))).await?;
        }
        self.bindings.lock().await.insert(key, NioBinding { local_port, nio_name: name });
        Ok(local_port)
    }

    pub async fn attach_nio(&self, vm: &DynamipsVm, adapter_number: u32, port_number: u32, local_port: u16, remote_port: u16) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        self.ensure_hypervisor(vm).await?;
        let key = binding_key(vm, adapter_number, port_number);
        if let Some(existing) = self.bindings.lock().await.get(&key) {
            if existing.local_port == local_port { return Ok(()); }
            return Err(RuntimeError::Failed("Dynamips adapter already has a different NIO binding".to_owned()));
        }
        let name = nio_name(vm, adapter_number, port_number);
        self.send(vm.hypervisor_port, &format!("nio create_udp {name} {local_port} 127.0.0.1 {remote_port}")).await?;
        if let Err(error) = self.send(vm.hypervisor_port, &format!("vm slot_add_nio_binding {} {adapter_number} {port_number} {name}", quote(&vm.name))).await {
            let _ = self.send(vm.hypervisor_port, &format!("nio delete {name}")).await;
            return Err(error);
        }
        if self.hypervisor_status(vm.hypervisor_port, &vm.name).await? == "started" {
            self.send(vm.hypervisor_port, &format!("vm slot_enable_nio {} {adapter_number} {port_number}", quote(&vm.name))).await?;
        }
        self.bindings.lock().await.insert(key, NioBinding { local_port, nio_name: name });
        Ok(())
    }

    pub async fn detach_nio(&self, vm: &DynamipsVm, adapter_number: u32, port_number: u32) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let Some(binding) = self.bindings.lock().await.remove(&binding_key(vm, adapter_number, port_number)) else { return Ok(()); };
        if self.router_process_alive(&vm.node_id).await {
            let _ = self.send(vm.hypervisor_port, &format!("vm slot_disable_nio {} {adapter_number} {port_number}", quote(&vm.name))).await;
            let _ = self.send(vm.hypervisor_port, &format!("vm slot_remove_nio_binding {} {adapter_number} {port_number}", quote(&vm.name))).await;
            let _ = self.send(vm.hypervisor_port, &format!("nio delete {}", binding.nio_name)).await;
        }
        Ok(())
    }

    pub async fn remove_switch_endpoint(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let bindings: Vec<_> = {
            let mut registry = self.bindings.lock().await;
            let keys: Vec<_> = registry.keys().filter(|(node_id, _, _)| node_id == &vm.node_id).cloned().collect();
            keys.into_iter().filter_map(|key| registry.remove(&key).map(|binding| (key, binding))).collect()
        };
        if self.router_process_alive(&vm.node_id).await {
            for ((_, adapter, port), binding) in bindings {
                let _ = self.send(vm.hypervisor_port, &format!("vm slot_disable_nio {} {adapter} {port}", quote(&vm.name))).await;
                let _ = self.send(vm.hypervisor_port, &format!("vm slot_remove_nio_binding {} {adapter} {port}", quote(&vm.name))).await;
                let _ = self.send(vm.hypervisor_port, &format!("nio delete {}", binding.nio_name)).await;
            }
        }
        Ok(())
    }
}
