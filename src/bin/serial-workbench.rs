use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use serial_mcp_server::workbench::{self, client::WorkbenchClient, Hub};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Shared serial capture, browser UI, scriptable CLI and MCP adapter"
)]
struct Cli {
    #[arg(long, global = true, default_value = "http://127.0.0.1:8765")]
    url: String,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Start the independent daemon and embedded web UI. Does not open any device.
    Serve {
        #[arg(long, default_value_t = 8765)]
        port: u16,
        #[arg(long, default_value = "workbench-data")]
        data_dir: PathBuf,
    },
    /// Serve MCP over stdio, forwarding to the running daemon.
    Mcp,
    Status,
    /// Stop the shared daemon, cancel tasks and flush all sessions.
    Stop,
    Ports,
    Sessions,
    /// Open a shared session. --demo never touches hardware.
    Open {
        #[arg(long, default_value = "DEMO")]
        port: String,
        #[arg(long, default_value_t = 115200)]
        baud: u32,
        #[arg(long, default_value = "8")]
        data_bits: String,
        #[arg(long, default_value = "1")]
        stop_bits: String,
        #[arg(long, default_value = "none")]
        parity: String,
        #[arg(long, default_value = "none")]
        flow_control: String,
        #[arg(long)]
        demo: bool,
    },
    /// Non-consuming incremental event query; --history reads disk instead of the ring.
    Events {
        session: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long)]
        contains: Option<String>,
        #[arg(long)]
        history: bool,
    },
    /// Send exactly one command through the shared queue.
    Send(Send),
    /// Run a JSON macro pack without interrupting shared capture.
    Macro {
        session: String,
        file: PathBuf,
        name: String,
        #[arg(long)]
        assembly: bool,
        #[arg(long)]
        wait: bool,
    },
    Tasks,
    Task {
        id: String,
        #[arg(long)]
        wait: bool,
    },
    Cancel {
        id: String,
    },
    /// Cancel pending host tasks and return control to the human. No Ctrl-C is sent.
    Takeover {
        session: String,
    },
    /// Release the manual-control lease so CLI/MCP tasks can submit again.
    Release {
        session: String,
    },
    /// Close the physical session for every viewer; not needed to detach.
    Close {
        session: String,
    },
    Control {
        session: String,
        #[arg(long)]
        rts: Option<bool>,
        #[arg(long)]
        dtr: Option<bool>,
    },
}
#[derive(Args)]
struct Send {
    session: String,
    /// Exact payload; alternatively use --file to avoid shell escaping or secret argv.
    #[arg(long, conflicts_with = "file", required_unless_present = "file")]
    data: Option<String>,
    #[arg(long, conflicts_with = "data")]
    file: Option<PathBuf>,
    #[arg(long, default_value = "utf8")]
    encoding: String,
    #[arg(long, value_enum, default_value = "none")]
    newline: Newline,
    #[arg(long)]
    wait_for: Option<String>,
    #[arg(long, default_value_t = 5000)]
    timeout_ms: u64,
    #[arg(long)]
    sensitive: bool,
    #[arg(long)]
    wait: bool,
}
#[derive(Clone, Copy, ValueEnum)]
enum Newline {
    None,
    Lf,
    Cr,
    #[value(name = "crlf")]
    CrLf,
}
fn session_id(id: &str) -> Result<String> {
    Ok(uuid::Uuid::parse_str(id)?.to_string())
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{}", json!({"error":error.to_string()}));
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Command::Serve { port, data_dir } = cli.command {
        return workbench::http::serve(Hub::new(data_dir).await?, port).await;
    }
    let client = WorkbenchClient::new(&cli.url)?;
    let mut wait = false;
    let value=match cli.command {
        Command::Serve{..}=>unreachable!(),
        Command::Mcp=>return workbench::mcp::serve(client).await,
        Command::Status=>client.get("/api/status").await?,
        Command::Stop=>client.post("/api/shutdown",json!({})).await?,
        Command::Ports=>client.get("/api/ports").await?,
        Command::Sessions=>client.get("/api/sessions").await?,
        Command::Open{port,baud,data_bits,stop_bits,parity,flow_control,demo}=>client.post("/api/sessions",json!({"port":port,"baud_rate":baud,"data_bits":data_bits,"stop_bits":stop_bits,"parity":parity,"flow_control":flow_control,"demo":demo})).await?,
        Command::Events{session,after,limit,contains,history}=>{
            let mut url=reqwest::Url::parse("http://localhost/")?;
            url.query_pairs_mut().append_pair("after",&after.to_string()).append_pair("limit",&limit.to_string());
            if let Some(s)=contains {url.query_pairs_mut().append_pair("contains",&s);}
            client.get(&format!("/api/sessions/{}/{}?{}",session_id(&session)?,if history {"history"} else {"events"},url.query().unwrap())).await?
        }
        Command::Send(a)=>{
            let mut data=if let Some(file)=a.file {tokio::fs::read_to_string(file).await?} else {a.data.unwrap()};
            if !matches!(a.newline,Newline::None) && a.encoding!="utf8" {bail!("--newline is only supported with utf8");}
            data.push_str(match a.newline {Newline::None=>"",Newline::Lf=>"\n",Newline::Cr=>"\r",Newline::CrLf=>"\r\n"});
            wait=a.wait;
            client.post(&format!("/api/sessions/{}/send",session_id(&a.session)?),json!({"data":data,"encoding":a.encoding,"source":"cli","wait_for":a.wait_for,"timeout_ms":a.timeout_ms,"sensitive":a.sensitive})).await?
        }
        Command::Macro{session,file,name,assembly,wait:w}=>{
            let pack:Value=serde_json::from_slice(&tokio::fs::read(file).await?)?; wait=w;
            client.post(&format!("/api/sessions/{}/macro",session_id(&session)?),json!({"pack":pack,"name":name,"assembly":assembly,"source":"cli"})).await?
        }
        Command::Tasks=>client.get("/api/tasks").await?,
        Command::Task{id,wait:w}=>{wait=w;client.get(&format!("/api/tasks/{}",session_id(&id)?)).await?}
        Command::Cancel{id}=>client.post(&format!("/api/tasks/{}/cancel",session_id(&id)?),json!({})).await?,
        Command::Takeover{session}=>client.post(&format!("/api/sessions/{}/takeover",session_id(&session)?),json!({})).await?,
        Command::Release{session}=>client.post(&format!("/api/sessions/{}/release",session_id(&session)?),json!({})).await?,
        Command::Close{session}=>client.post(&format!("/api/sessions/{}/close",session_id(&session)?),json!({})).await?,
        Command::Control{session,rts,dtr}=>client.post(&format!("/api/sessions/{}/control",session_id(&session)?),json!({"rts":rts,"dtr":dtr,"source":"cli"})).await?,
    };
    let value = if wait {
        client
            .wait_task(
                value["id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing task id"))?,
            )
            .await?
    } else {
        value
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    if wait && matches!(value["status"].as_str(), Some("failed" | "cancelled")) {
        std::process::exit(2);
    }
    Ok(())
}
