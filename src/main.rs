use std::{
    process::{Child, Command, Stdio},
    time::Duration,
    thread::sleep,
};
use wait_timeout::ChildExt; // for .wait_timeout()


use ctrlc;
use derive_new::new;

#[derive(new)]
struct Tunnel {
    service: String,
    port: u16,
    #[new(default)]
    child: Option<Child>,
}

impl Tunnel {
    fn start(&mut self)  {
        let mut cmd = Command::new("b2c2");
        cmd.arg("tunnel")
            .arg("-e")
            .arg("prod")
            .arg("-s")
            .arg(&self.service)
            .arg("-p")
            .arg(self.port.to_string())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let child = cmd
            .spawn()
            .unwrap();
        self.child = Some(child);
    }

    fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let res = child.kill();
            println!("kill res: {res:?}")
        }
    }

    fn alive(&mut self) -> bool {
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    println!("Process exited with: {}", status);
                    false
                }
                Ok(None) => {
                    //println!("Process is still running");
                    true
                }
                Err(e) => {
                    println!("Error checking process: {}", e);
                    false
                }
            }
        } else {
            false
        }
    }
}

fn login() {
    println!("🔐 Logging in to B2C2 AWS...");
    let status = Command::new("b2c2")
        .arg("aws")
        .arg("login")
        .status()
        .expect("Failed to login to B2C2 AWS");
    if !status.success() {
        eprintln!("❌ AWS login failed");
        std::process::exit(1);
    }
    println!("✅ Logged in successfully");
}

fn is_logged_in() -> bool {
    let output = Command::new("b2c2")
        .args(["aws", "auth", "status"])
        .output()
        .unwrap()
        .stderr;
    let output = String::from_utf8_lossy(&output);
    println!("{output}");
    !output.contains("not logged in")
}

fn auth_status() {
    let _ = Command::new("b2c2")
        .args(["aws", "auth", "status"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
}

#[derive(Clone, Copy, Debug)]
enum ServiceType {
    Redis,
    Postgres,
}

#[derive(new)]
struct Service {
    service_type: ServiceType,
    port: u16
}


impl Service {

    fn ping(&self) -> bool {
        let mut child = self.spawn();

        let timeout = Duration::from_secs(10);

        match child.wait_timeout(timeout).unwrap() {
            Some(status) => {
                println!("{:?} {:?}", self.service_type, status);
                status.success()
            }
            None => {
                println!("ping {:?} timed out", self.service_type);
                // Kill the process if it’s still running
                let _ = child.kill();
                let _ = child.wait();
                false
            }
        }
    }

    fn spawn(&self) -> Child {
        match self.service_type {
            ServiceType::Postgres => {
                //println!("pinging postgres");
                Command::new("psql")
                    .arg("-h")
                    .arg("localhost")
                    .arg("-p")
                    .arg(self.port.to_string())
                    .arg("-c")
                    .arg("SELECT 1")
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .expect("failed to spawn process")
            },
            ServiceType::Redis => {
                //println!("pinging redis");
                Command::new("redis-cli")
                    .arg("-p")
                    .arg(self.port.to_string())
                    .arg("-t")
                    .arg("10")
                    .arg("ping")
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .expect("failed to spawn process")
            },
        }
    }
}

fn run(tunnels: &Vec<(&str, u16, ServiceType)>) {
    let (mut tunnels, services): (Vec<Tunnel>, Vec<Service>)  = tunnels.iter().map(|(service, port, service_type)| {
        let mut tunnel = Tunnel::new(service.to_string(), *port);
        let service = Service::new(*service_type, *port);
        tunnel.start();
        (tunnel, service)
    }).unzip();
    sleep(Duration::from_secs(20));
    let mut alive = true;
    while alive {
        sleep(Duration::from_secs(5));
        for i in 0..tunnels.len() {
            if !tunnels[i].alive() {
                println!("tunnel task died");
                alive = false;
                break;
            }
        }
        if !services.iter().map(Service::ping).all(|x|x) {
            println!("ping failed!");
            alive = false;
        }
    }
    println!("cleaning up tunnel tasks");
    for mut tunnel in tunnels {
        tunnel.kill();
    }
    println!("cleanup done");
}

fn kill_tunnels() {
    let _ = Command::new("killall")
        .args(["session-manager-plugin"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
}


fn main() {
    let tunnels = vec![
        ("primaryredisreplica", 6001, ServiceType::Redis),
        ("secondaryredisreplica", 6002, ServiceType::Redis),
        ("optionsredisreplica", 6003, ServiceType::Redis),
        ("optionsconfigdb", 5608, ServiceType::Postgres),
    ];
    loop {
        kill_tunnels();
        if !is_logged_in() {
            login();
        }
        auth_status();
        run(&tunnels);
    }
}
