
# Firebender: Web-Based Firecracker Workstation Service

Firebender is a web-based service designed to create, manage, and interact with isolated KVM-based microVMs using **Firecracker** and **Rust**. It provides a simple REST API and a real-time web UI, featuring a built-in terminal for a seamless sandboxed workstation experience.

The tool provides a complete lifecycle for virtual workstations:

- ```Dynamic Creation```: Creates workstations on-demand with user-specified vCPU, memory, SMT, and bandwidth limit configurations.

- ```Complete Isolation```: Uses separate filesystems and kernels for each non-read-only workstation, so each workstation is completely separate from each other.

- ```Network Management```: Automatically creates and manages a network bridge and TAP devices to provide network connectivity for each workstation.

- ```Interactive Terminal```: Provides a real-time, in-browser terminal connected directly to the workstation's SSH shell via a WebSocket bridge.

**Note 1**: The service requires _firecracker_, _sshpass_, and standard Linux networking tools (like _ip_) to be installed on the host system.

**Note 2**: The service depends on a pre-configured Linux kernel image (```kernel.bin```) and a root filesystem image (```rootfs.ext4```). The rootfs must contain a running SSH server and be configured with a known username and password (for example, **root**/**root**) for the terminal feature to work.

**Note 3**: Because the application creates and manages network devices (Linux bridges and TAP interfaces), the backend server must have **root** privileges.

**Note 4**: Upon starting, the application automatically creates a network bridge named ```fc-br0``` on the host and assigns IP addresses to workstations from the ```172.16.0.0/24``` subnet. This may conflict with existing network configurations on the host machine.

**Note 5**: The current state of active workstations is stored in-memory. Restarting the backend server will lose track of all running VMs.

**Note 6**: To enable an SSH connection to the server, SSH configurations must be made to the filesystem. Additionally, if the server needs to be able to access the internet, the following commands must be run.
```
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -A POSTROUTING -o ens33 -j MASQUERADE
```

**Note 7**: To start SSH, data must be written to some files, but this cannot be done on read-only workstations. Therefore, the following script has been added to the ```/sbin/readonly-init``` file in the filesystem. This allows SSH connections to be established on read-only workstations.
```
#!/bin/sh
# Mount necessary tmpfs directories for SSH
mount -t tmpfs tmpfs /tmp
mount -t tmpfs tmpfs /var/run
mount -t tmpfs tmpfs /var/log

# Create directory for SSH
mkdir -p /var/run/sshd
chmod 755 /var/run/sshd

# Start SSH daemon
/usr/sbin/sshd

# Continue with normal boot
exec /sbin/init
```

The flow of the program is as follows.

### 1. API Request & VM Preparation

When a **POST** request with a JSON payload is sent to the ```/workstations``` endpoint, the Axum web server's handler validates the input. It generates a unique UUID for the new workstation, determines the next available IP address, and constructs a _Workstation_ struct with the properties specified by the user (RAM, CPU, etc.). 

### 2. Host & Network Configuration

The handler proceeds to execute a series of system commands. A dedicated TAP network interface is created on the host and attached to the ```fc-br0``` bridge. If the _read_only_ flag is **false**, the base ```kernel.bin``` and ```rootfs.ext4``` images are copied to new files using the workstation's UUID to give it a persistent, writable disk. If _read_only_ is **true**, the base images are used directly.

### 3. Firecracker Lifecycle Management

A firecracker process is spawned in a new thread, listening on a unique Unix domain socket. The backend then uses ```curl``` commands directed at this socket to configure the microVM. It sets the machine configuration (RAM, CPU, etc.), attaches the kernel and rootfs drives, configures the network interface with any specified bandwidth limits, and finally sends the command to start the instance.

### 4. Terminal Access

Upon a WebSocket connection to the ```/ws/workstations/{id}/terminal endpoint```, the handler retrieves the workstation's IP and spawns an **sshpass** process to automatically log into the VM. A bidirectional proxy is then established to pipe data between the WebSocket and the SSH process’s standard I/O streams (```stdin```, ```stdout```, ```stderr```). This effectively connects the user's browser directly to the VM's shell. The sshpass process is automatically killed when the session terminates.

### How to Run

firebender:
```
cd firebender
sudo RUST_LOG=info cargo run
```

firebender-ui:
```
cd firebender-ui
python3 -m http.server 8000
```

Then, you can start using the application from the web interface by going to ```http://127.0.0.1:8000```.