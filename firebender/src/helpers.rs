use crate::{
    AppState,
    Workstation,
    IMAGE_PATH,
    KERNEL_IMAGE_PATH,
    ROOTFS_IMAGE_PATH,
};

use tracing::{
    info,
    error,
};

use std::{
    process::Command,
    fs,
    thread,
    time::Duration,
};

/*----------------------------------------------------------HELPERS----------------------------------------------------------*/

fn delete_tap(tap_num: u32) -> Result<(), String> {
    let tap = format!("fc-tap{}", tap_num);
    info!("Deleting tap device: {}", &tap);
    let delete_tap = Command::new("sudo")
        .args(["ip", "tuntap", "del", "dev", &tap, "mode", "tap"])
        .status();

    if delete_tap.is_err() || !delete_tap.unwrap().success() {
        let err_msg = format!("Failed to delete tap device: {}", tap);
        return Err(err_msg);
    }

    info!("Deleted tap device: {}", tap);

    Ok(())
}

pub fn cleanup_taps(app_state: &AppState) -> Result<(), String> {
    let vm_count = app_state.vm_counter.lock().unwrap();
    let workstations = app_state.workstations.lock().unwrap();

    for i in 2..*vm_count {
        if let Err(e) = delete_tap(i - 2) {
            return Err(e);
        }

        if let Some(workstation) = workstations.values().find(|w| w.order == i) {
            if workstation.read_only {
                info!("VM ID: {} is read-only. Skipping disk cleanup.", workstation.id);
            }
            else {
                let kernel = format!("{}kernel-{}.bin", IMAGE_PATH, workstation.id);
                let fs = format!("{}rootfs-{}.ext4", IMAGE_PATH, workstation.id);
                
                let delete_kernel = fs::remove_file(&kernel); 
                if delete_kernel.is_err() {
                    let err_msg = format!("Failed to delete kernel image for VM ID: {}", workstation.id);
                    error!("{}", err_msg);
                }

                let delete_fs = fs::remove_file(&fs);
                if delete_fs.is_err() {
                    let err_msg = format!("Failed to delete root filesystem for VM ID: {}", workstation.id);
                    error!("{}", err_msg);
                }
            }
        }
    }

    Ok(())
}

pub fn create_bridge() -> Result<(), String> {

    info!("Checking for fc-br0 network bridge...");

    let check_bridge = Command::new("ip")
        .args(["link", "show", "fc-br0"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if check_bridge.is_ok() && check_bridge.unwrap().success() {
        info!("fc-br0 bridge already exists. Skipping creation.");
        return Ok(());
    }

    info!("fc-br0 not found. Creating bridge...");

    let create_bridge = Command::new("sudo")
        .args(["ip", "link", "add", "name", "fc-br0", "type", "bridge"])
        .status();

    if create_bridge.is_err() || !create_bridge.unwrap().success() {
        let err_msg = "Failed to create fc-br0 bridge.".to_string();
        error!("{}", err_msg);
        return Err(err_msg);
    }

    let set_up_bridge = Command::new("sudo")
        .args(["ip", "addr", "add", "172.16.0.1/24", "dev", "fc-br0"])
        .status();

    if set_up_bridge.is_err() || !set_up_bridge.unwrap().success() {
        let err_msg = "Failed to set up fc-br0 bridge.".to_string();
        error!("{}", err_msg);
        return Err(err_msg);
    }

    let bring_up_bridge = Command::new("sudo")
        .args(["ip", "link", "set", "dev", "fc-br0", "up"])
        .status();

    if bring_up_bridge.is_err() || !bring_up_bridge.unwrap().success() {
        let err_msg = "Failed to bring up fc-br0 bridge.".to_string();
        error!("{}", err_msg);
        return Err(err_msg);
    }

    Ok(())
}

pub fn connect_vms_to_network(vm_count: u32) -> Result<(), String> {
    let tap = format!("fc-tap{}", vm_count - 2);
    let create_tap = Command::new("sudo")
        .args(["ip", "tuntap", "add", "dev", &tap, "mode", "tap"])
        .status();

    if create_tap.is_err() || !create_tap.unwrap().success() {
        let err_msg = format!("Failed to create tap interface: {}", tap);
        error!("{}", err_msg);
        return Err(err_msg);
    }

    let set_up_tap = Command::new("sudo")
        .args(["ip", "link", "set", &tap, "master", "fc-br0"])
        .status();

    if set_up_tap.is_err() || !set_up_tap.unwrap().success() {
        let err_msg = format!("Failed to attach tap device to bridge: {}", tap);
        error!("{}", err_msg);
        return Err(err_msg);
    }

    let bring_up_tap = Command::new("sudo")
        .args(["ip", "link", "set", &tap, "up"])
        .status();

    if bring_up_tap.is_err() || !bring_up_tap.unwrap().success() {
        let err_msg = format!("Failed to bring up tap device: {}", tap);
        error!("{}", err_msg);
        return Err(err_msg);
    }

    info!("Successfully created and configured tap device: {}", tap);

    Ok(())
}

pub fn spawn_firecracker_process(vm_id: &str) -> Result<String, String> {
    let socket_path = format!("/tmp/firecracker-{}.socket", vm_id);

    let _ = fs::remove_file(&socket_path);

    let vm_id = vm_id.to_string();
    let socket_path_thread = socket_path.clone();

    thread::spawn(move || {
        info!("Spawning Firecracker process with socket: {}", &socket_path_thread);
        
        let mut start_firecracker = match Command::new("firecracker")
            .arg("--api-sock")
            .arg(&socket_path_thread)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn() {
                Ok(child) => child,
                Err(e) => {
                    let err_msg = format!("Failed to spawn Firecracker process: {}", e);
                    error!("{}", err_msg);
                    return;
                }
            };
        
        
        match start_firecracker.wait() {
            Ok(status) => {
                if status.success() {
                    info!("Firecracker process for VM ID: {} exited successfully.", vm_id);
                } else {
                    error!("Firecracker process for VM ID: {} exited with status: {}", vm_id, status);
                }
            },
            Err(e) => {
                let err_msg = format!("Failed to wait on Firecracker process: {}", e);
                error!("{}", err_msg);
                return;
            }
        }

        let _ = fs::remove_file(&socket_path_thread);
        info!("Cleaned up socket: {}", &socket_path_thread);
    });

    thread::sleep(Duration::from_millis(50));

    Ok(socket_path)
}

pub fn configure_vm(socket_path: &str, vm_id: &str, ip_addr: &str, current_vm_counter: u32, 
    vcpu_count: u64, mem_size_mib: u32, smt_enabled: bool, read_only: bool, bandwidth: u64) -> Result<(), String> {
    
    let set_machine_cfg = format!(
        r#"{{"vcpu_count": {}, "mem_size_mib": {}, "smt": {}}}"#,
        vcpu_count, mem_size_mib, smt_enabled
    );

    info!("Configuring VM Kernel ID: {} with {} vCPUs, {} MiB RAM, SMT: {}, IP: {}", vm_id, vcpu_count, mem_size_mib, smt_enabled, ip_addr);

    let configure_machine = Command::new("curl")
        .args(["--unix-socket", &socket_path, "-X", "PUT", "http://localhost/machine-config", "-d", &set_machine_cfg, "-H", "Content-Type: application/json"])
        .status();

    if configure_machine.is_err() || !configure_machine.unwrap().success() {
        let err_msg = format!("Failed to configure machine for VM ID: {}", vm_id);
        error!("{}", err_msg);
        return Err(err_msg);
    }

    let boot_args = format!(
        "console=ttyS0 reboot=k panic=1 pci=off ip={}::172.16.0.1:255.255.255.0::eth0:on i8042.noaux i8042.nomux i8042.nopnp i8042.dumbkbd",
        ip_addr
    );

    let kernel_path = if read_only {
        format!("{}", KERNEL_IMAGE_PATH)
    } else {
        let customized_kernel = format!("{}kernel-{}.bin", IMAGE_PATH, vm_id);

        let copy_kernel = fs::copy(KERNEL_IMAGE_PATH, &customized_kernel);

        if copy_kernel.is_err() {
            let err_msg = format!("Failed to copy kernel image for VM ID: {}", vm_id);
            error!("{}", err_msg);
            return Err(err_msg);
        }

        customized_kernel
    };

    let set_kernel = format!(
        r#"{{"kernel_image_path": "{}", "boot_args": "{}"}}"#,
        kernel_path, boot_args
    );

    let configure_kernel = Command::new("curl")
        .args(["--unix-socket", &socket_path, "-X", "PUT", "http://localhost/boot-source", "-d", &set_kernel, "-H", "Content-Type: application/json"])
        .status();
    
    if configure_kernel.is_err() || !configure_kernel.unwrap().success() {
        let err_msg = format!("Failed to configure kernel for VM ID: {}", vm_id);
        error!("{}", err_msg);
        return Err(err_msg);
    }

    let rootfs_path = if read_only {
        format!("{}", ROOTFS_IMAGE_PATH)
    } else {
        let customized_rootfs = format!("{}rootfs-{}.ext4", IMAGE_PATH, vm_id);

        let copy_fs = fs::copy(ROOTFS_IMAGE_PATH, &customized_rootfs);

        if copy_fs.is_err() {
            let err_msg = format!("Failed to copy root filesystem for VM ID: {}", vm_id);
            error!("{}", err_msg);
            return Err(err_msg);
        }

        customized_rootfs
    };

    let set_fs = format!(
        r#"{{"drive_id": "rootfs", "path_on_host": "{}", "is_root_device": true, "is_read_only": {}}}"#,
        rootfs_path, read_only
    );

    info!("Configuring VM RootFS ID: {} as Read-Only: {}", vm_id, read_only);

    let configure_fs = Command::new("curl")
        .args(["--unix-socket", &socket_path, "-X", "PUT", "http://localhost/drives/rootfs", "-d", &set_fs, "-H", "Content-Type: application/json"])
        .status();
    
    if configure_fs.is_err() || !configure_fs.unwrap().success() {
        let err_msg = format!("Failed to configure root filesystem for VM ID: {}", vm_id);
        error!("{}", err_msg);
        return Err(err_msg);
    }

    let configure_network = if bandwidth > 0 {

        let bps = bandwidth * 125000; // Mbits to Bytes

        format!(
            r#"{{"iface_id": "eth0", "host_dev_name": "fc-tap{}", "tx_rate_limiter": {{"bandwidth": {{"size": {}, "refill_time": 1000}}}}, "rx_rate_limiter": {{"bandwidth": {{"size": {}, "refill_time": 1000}}}}}}"#,
            current_vm_counter - 2,
            bps,
            bps
        )
        
    } else {
        format!(
            r#"{{"iface_id": "eth0", "host_dev_name": "fc-tap{}"}}"#,
            current_vm_counter - 2
        )
    };

    let set_network = Command::new("curl")
        .args(["--unix-socket", &socket_path, "-X", "PUT", "http://localhost/network-interfaces/eth0", "-d", &configure_network, "-H", "Content-Type: application/json"])
        .status();
    
    if set_network.is_err() || !set_network.unwrap().success() {
        let err_msg = format!("Failed to configure network for VM ID: {}", vm_id);
        error!("{}", err_msg);
        return Err(err_msg);
    }

    let start_vm = Command::new("curl")
        .args(["--unix-socket", &socket_path, "-X", "PUT", "http://localhost/actions", "-d", r#"{"action_type": "InstanceStart"}"#, "-H", "Content-Type: application/json"])
        .status();
    
    if start_vm.is_err() || !start_vm.unwrap().success() {
        let err_msg = format!("Failed to start VM ID: {}", vm_id);
        error!("{}", err_msg);
        return Err(err_msg);    
    }

    Ok(())
}

pub fn shutdown_vm(workstation: &Workstation) -> Result<(), String> {
    info!("Shutting down VM ID: {}", workstation.id);

    let socket_path = format!("/tmp/firecracker-{}.socket", workstation.id);

    let shutdown_vm = Command::new("curl")
        .args(["--unix-socket", &socket_path, "-X", "PUT", "http://localhost/actions", "-d", r#"{"action_type": "SendCtrlAltDel"}"#, "-H", "Content-Type: application/json"])
        .status();

    if std::path::Path::new(&socket_path).exists() {
        if shutdown_vm.is_err() || !shutdown_vm.unwrap().success() {
            let err_msg = format!("Failed to send shutdown signal to VM ID: {}", workstation.id);
            error!("{}", err_msg);
            return Err(err_msg);    
        }
    }

    if workstation.read_only {
        info!("VM ID: {} is read-only. Skipping disk cleanup.", workstation.id);
    }
    else {
        let kernel = format!("{}kernel-{}.bin", IMAGE_PATH, workstation.id);
        let fs = format!("{}rootfs-{}.ext4", IMAGE_PATH, workstation.id);
        
        let delete_kernel = fs::remove_file(&kernel); 
        if delete_kernel.is_err() {
            let err_msg = format!("Failed to delete kernel image for VM ID: {}", workstation.id);
            error!("{}", err_msg);
        }

        let delete_fs = fs::remove_file(&fs);
        if delete_fs.is_err() {
            let err_msg = format!("Failed to delete root filesystem for VM ID: {}", workstation.id);
            error!("{}", err_msg);
        }
    }

    thread::sleep(Duration::from_secs(5));

    if let Err(e) = delete_tap(workstation.order - 2) {
        error!("{}", e);
        return Err(e);
    }

    Ok(())
}