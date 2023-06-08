#![allow(dead_code)]
use common::{forward, Message};
use std::error::Error;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    let args = Args::default();
    args.echo();
    for i in 1..=args.reconnect {
        if let Err(e) = args.run().await {
            println!("{e}")
        }
        println!("第 {i} 次重连")
    }
    println!("与服务器连失败")
}

#[derive(Debug, Clone)]
struct Args {
    /// 本地地址
    local_addr: SocketAddr,
    /// 连接地址
    connect_addr: SocketAddr,
    /// 转发地址
    forward_addr: SocketAddr,
    /// 重连次数
    reconnect: usize,
    /// 心跳包
    heartbeat: Duration,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            local_addr: "127.0.0.1:3000".parse().unwrap(),
            connect_addr: "127.0.0.1:10087".parse().unwrap(),
            forward_addr: "127.0.0.1:10088".parse().unwrap(),
            reconnect: 5,
            heartbeat: Duration::from_secs(30),
        }
    }
}

impl Args {
    async fn run(&self) -> Result<(), Box<dyn Error>> {
        let ts = TcpStream::connect(self.connect_addr)
            .await
            .map_err(|e| format!("与服务器连接失败 -> {:<15} 原因:{e}", self.connect_addr))?;
        let (read, mut write) = ts.into_split();

        let mut bs = BufReader::new(read);
        println!("与服务器连接成功 <- {}", self.connect_addr);
        let duration = self.heartbeat;
        let ping = Message::Ping.to_string();

        // 心跳包
        let task = tokio::spawn(async move {
            loop {
                sleep(duration).await;
                if let Err(e) = write.write(ping.as_bytes()).await {
                    println!("发送 ping 数据失败: {e}")
                }
            }
        });

        loop {
            let mut line = String::new();
            if let Err(e) = bs.read_line(&mut line).await {
                task.abort();
                println!("与服务器连接断开 -> {:<15} 原因:{e}", self.connect_addr);
                break;
            };
            //  收到 new 消息新建转发连接
            if let Message::New = Message::from(line.as_str()) {
                self.forward().await?
            };
        }
        Ok(())
    }

    async fn forward(&self) -> Result<(), Box<dyn Error>> {
        let local = TcpStream::connect(self.local_addr)
            .await
            .map_err(|e| format!("本地服务连接失败 -> {:<15} 原因:{e}", self.local_addr))?;

        let remote = TcpStream::connect(self.forward_addr)
            .await
            .map_err(|e| format!("远程服务连接失败 -> {:<15} 原因:{e}", self.forward_addr))?;

        forward(local, remote).await;
        Ok(())
    }

    fn echo(&self) {
        println!("本地地址:{}", self.local_addr);
        println!("连接地址:{}", self.connect_addr);
        println!("转发地址:{}", self.forward_addr);
    }
}
