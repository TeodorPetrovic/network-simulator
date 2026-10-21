use super::*;

impl DynamipsRuntime {
    pub async fn switch_endpoint(
        &self,
        vm: &DynamipsVm,
        adapter_number: u32,
        port_number: u32,
        switch_port: u16,
    ) -> Result<u16, RuntimeError> {
        // TODO(RUNTIME-NET Dynamips adapter hook): bind all configured router
        // module ports through NET-managed NIOs; this runtime API currently
        // exposes one adapter/port and must not implement attachment itself.
        let _guard = self.operation_lock.lock().await;
        if adapter_number != 0 || port_number != 0 {
            return Err(RuntimeError::Failed(
                "This Dynamips template exposes only adapter 0, port 0".to_owned(),
            ));
        }
        self.ensure_hypervisor(vm).await?;
        if let Some(port) = self.bindings.lock().await.get(&vm.node_id).copied() {
            return Ok(port);
        }
        let local_port = available_udp_port().await?;
        let nio_name = format!("udp-{}", vm.node_id.replace('-', ""));
        self.send(
            vm.hypervisor_port,
            &format!("nio create_udp {nio_name} {local_port} 127.0.0.1 {switch_port}"),
        )
        .await?;
        self.send(
            vm.hypervisor_port,
            &format!("vm slot_add_nio_binding {} 0 0 {nio_name}", quote(&vm.name)),
        )
        .await?;
        if self.hypervisor_status(vm.hypervisor_port, &vm.name).await? == "started" {
            self.send(
                vm.hypervisor_port,
                &format!("vm slot_enable_nio {} 0 0", quote(&vm.name)),
            )
            .await?;
        }
        self.bindings
            .lock()
            .await
            .insert(vm.node_id.clone(), local_port);
        Ok(local_port)
    }

    pub async fn attach_nio(
        &self,
        vm: &DynamipsVm,
        adapter_number: u32,
        port_number: u32,
        local_port: u16,
        remote_port: u16,
    ) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        if adapter_number != 0 || port_number != 0 {
            return Err(RuntimeError::Failed(
                "This Dynamips template exposes only adapter 0, port 0".to_owned(),
            ));
        }
        self.ensure_hypervisor(vm).await?;
        if let Some(existing) = self.bindings.lock().await.get(&vm.node_id).copied() {
            if existing == local_port {
                return Ok(());
            }
            return Err(RuntimeError::Failed(
                "Dynamips adapter already has a different NIO binding".to_owned(),
            ));
        }
        let nio_name = format!("udp-{}", vm.node_id.replace('-', ""));
        self.send(
            vm.hypervisor_port,
            &format!("nio create_udp {nio_name} {local_port} 127.0.0.1 {remote_port}"),
        )
        .await?;
        if let Err(error) = self
            .send(
                vm.hypervisor_port,
                &format!(
                    "vm slot_add_nio_binding {} {adapter_number} {port_number} {nio_name}",
                    quote(&vm.name)
                ),
            )
            .await
        {
            let _ = self
                .send(vm.hypervisor_port, &format!("nio delete {nio_name}"))
                .await;
            return Err(error);
        }
        if self.hypervisor_status(vm.hypervisor_port, &vm.name).await? == "started" {
            self.send(
                vm.hypervisor_port,
                &format!(
                    "vm slot_enable_nio {} {adapter_number} {port_number}",
                    quote(&vm.name)
                ),
            )
            .await?;
        }
        self.bindings
            .lock()
            .await
            .insert(vm.node_id.clone(), local_port);
        Ok(())
    }

    pub async fn detach_nio(
        &self,
        vm: &DynamipsVm,
        adapter_number: u32,
        port_number: u32,
    ) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        if adapter_number != 0 || port_number != 0 {
            return Ok(());
        }
        let Some(_binding) = self.bindings.lock().await.remove(&vm.node_id) else {
            return Ok(());
        };
        if self.router_process_alive(&vm.node_id).await {
            let nio_name = format!("udp-{}", vm.node_id.replace('-', ""));
            let _ = self
                .send(
                    vm.hypervisor_port,
                    &format!(
                        "vm slot_disable_nio {} {adapter_number} {port_number}",
                        quote(&vm.name)
                    ),
                )
                .await;
            let _ = self
                .send(
                    vm.hypervisor_port,
                    &format!(
                        "vm slot_remove_nio_binding {} {adapter_number} {port_number}",
                        quote(&vm.name)
                    ),
                )
                .await;
            let _ = self
                .send(vm.hypervisor_port, &format!("nio delete {nio_name}"))
                .await;
        }
        Ok(())
    }

    pub async fn remove_switch_endpoint(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let binding = self.bindings.lock().await.remove(&vm.node_id);
        if self.router_process_alive(&vm.node_id).await {
            if binding.is_none() {
                return Ok(());
            }
            let nio_name = format!("udp-{}", vm.node_id.replace('-', ""));
            let _ = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm slot_disable_nio {} 0 0", quote(&vm.name)),
                )
                .await;
            let _ = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm slot_remove_nio_binding {} 0 0", quote(&vm.name)),
                )
                .await;
            let _ = self
                .send(vm.hypervisor_port, &format!("nio delete {nio_name}"))
                .await;
        }
        Ok(())
    }
}
