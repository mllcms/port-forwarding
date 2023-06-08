#![allow(dead_code)]
use common::{forward, Message};
use std::time::Duration;
use std::{error::Error, net::SocketAddr, sync::Arc};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::Receiver;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::{
        mpsc::{self, Sender},
        oneshot, RwLock,
    },
    time::sleep,
};

#[tokio::main]
async fn main() {
    if let Err(e) = Service::default().run().await {
        println!("{e}")
    }
}

struct Service {
    conn: Arc<RwLock<Option<Sender<Message>>>>,
    forward_conn: Arc<Forward>,
    args: Arc<Args>,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            conn: Arc::new(Default::default()),
            forward_conn: Arc::new(Forward::new(5000)),
            args: Arc::new(Args::default()),
        }
    }
}

impl Service {
    async fn run(&self) -> Result<(), Box<dyn Error>> {
        tokio::select! {
            res = self.main_service() => {
                println!("主服务异常");
                res
            }
            res = self.conn_service() => {
                println!("连接服务异常:");
                res
            },
            res = self.forward_service() => {
                println!("转发服务异常");
                res
            }
        }
    }

    /// 主服务
    async fn main_service(&self) -> Result<(), Box<dyn Error>> {
        let listener = TcpListener::bind(&self.args.server_addr).await?;
        loop {
            let (from, _) = listener.accept().await?;
            match self.conn.read().await.clone() {
                Some(sender) => {
                    sender.send(Message::New).await.unwrap();
                    let fc = self.forward_conn.clone();
                    match fc.tcp_stream().await {
                        Some(to) => forward(from, to).await,
                        None => println!("获取转发连接超时"),
                    };
                }
                None => {
                    println!("没有客户端连接");
                    continue;
                }
            };
        }
    }

    /// 连接服务
    async fn conn_service(&self) -> Result<(), Box<dyn Error>> {
        let args = self.args.clone();
        let listener = TcpListener::bind(args.connect_addr).await?;
        self.args.echo();

        loop {
            let (socket, addr) = listener.accept().await?;
            if self.conn.read().await.is_some() {
                println!("已有客户端连接,新连接无法加入 <- {}", addr);
                continue;
            }
            self.handle_conn(socket, addr).await
        }
    }

    /// 处理连接
    async fn handle_conn(&self, socket: TcpStream, addr: SocketAddr) {
        let (tx, mut rx) = mpsc::channel::<Message>(100);
        let (read, mut write) = socket.into_split();
        let conn = self.conn.clone();
        println!("新客户端加入连接 <- {}", addr);
        *conn.write().await = Some(tx.clone());

        // 获取心跳包
        tokio::spawn(async move {
            let mut read = BufReader::new(read);
            loop {
                let mut buf = String::new();
                if let Err(e) = read.read_line(&mut buf).await {
                    println!("与客户端连接断开 -> {:<15} 原因:{e}", addr);
                    *conn.write().await = None;
                    break;
                }
                if let Message::Ping = Message::from(buf.as_str()) {
                    if let Err(e) = tx.send(Message::Pong).await {
                        println!("连接信道异常: {e}")
                    }
                }
            }
        });

        // 与客户端通信 用户访问的时候发送 new 新建 tcp 连接
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = write.write(msg.to_string().as_bytes()).await {
                    println!("与客户端通信失败: {e}");
                    break;
                };
            }
        });
    }

    /// 转发服务
    async fn forward_service(&self) -> Result<(), Box<dyn Error>> {
        let listener = TcpListener::bind(self.args.forward_addr).await?;
        loop {
            let (socket, _) = listener.accept().await?;
            let sender = self.forward_conn.sender.clone();
            if let Err(e) = sender.send(socket).await {
                println!("转发服务信道异常: {e}");
            };
        }
    }
}

#[derive(Debug, Clone)]
struct Args {
    /// 服务地址
    server_addr: SocketAddr,
    /// 连接地址
    connect_addr: SocketAddr,
    /// 转发地址
    forward_addr: SocketAddr,
}
impl Args {
    fn echo(&self) {
        println!("服务地址:{}", self.server_addr);
        println!("连接地址:{}", self.connect_addr);
        println!("转发地址:{}", self.forward_addr);
    }
}

impl Default for Args {
    fn default() -> Self {
        Self {
            server_addr: "127.0.0.1:10086".parse().unwrap(),
            connect_addr: "127.0.0.1:10087".parse().unwrap(),
            forward_addr: "127.0.0.1:10088".parse().unwrap(),
        }
    }
}

/// 转发链接
struct Forward {
    /// 装填 tcp
    sender: Sender<TcpStream>,
    /// 获取 tcp
    take: Sender<oneshot::Sender<TcpStream>>,
    /// 超时
    timeout: Duration,
}

impl Forward {
    /// timeout 单位 ms
    fn new(timeout: u64) -> Self {
        let (sender, mut rx) = mpsc::channel(100);
        let (take, mut rx2) = mpsc::channel::<oneshot::Sender<TcpStream>>(100);

        // 持续接收 tcp 连接
        tokio::spawn(async move {
            while let Some(ts) = rx.recv().await {
                Self::load(ts, &mut rx2).await
            }
        });

        Self {
            sender,
            take,
            timeout: Duration::from_millis(timeout),
        }
    }

    /// 装填 TcpStream 超过 10ms 将被丢弃
    async fn load(ts: TcpStream, rx: &mut Receiver<oneshot::Sender<TcpStream>>) {
        let task = async {
            while let Some(s) = rx.recv().await {
                if s.is_closed() {
                    continue;
                }
                s.send(ts).ok();
                break;
            }
        };

        tokio::select! {
            _ = task =>{},
            _ = sleep(Duration::from_millis(10)) => {}
        }
    }

    /// 获取 TcpStream 超时返回 None
    async fn tcp_stream(&self) -> Option<TcpStream> {
        let (tx, rx) = oneshot::channel();
        self.take.send(tx).await.unwrap();

        tokio::select! {
            v = rx => match v {
                Ok(v) => Some(v),
                _ => None,
            },
            _ = sleep(self.timeout) => {
                None
            }
        }
    }
}
