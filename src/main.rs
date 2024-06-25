use std::io::{stdin, Read};
type Cap = i16;
type NodeIdx = i16;
type Flow = i16;
type EdgeIdx = u32;
type Height = u16;

use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let mut file = String::new();
    stdin().read_to_string(&mut file).unwrap();

    let (nodes, edges, _, _) = parse(file);

    // for the nodes
    let mut snds = vec![];
    let mut rcvs = vec![];
    for _ in 0..nodes.len() {
        let (a, b) = mpsc::channel(1000);
        snds.push(a);
        rcvs.push(b);
    }
    let sourcer = mpsc::channel(100);
    let targeter = mpsc::channel(100);
    let mut source_start_excess = 0;
    // for each node, look at it's neighbors and create Actor from it
    for (i, (node, receiver)) in nodes.iter().zip(rcvs.into_iter()).enumerate() {
        let mut actor = Actor {
            receiver,
            neighbors: vec![],
            excess_flow: 0,
            height: 0,
            responses: 0,
            pushed_to_anyone_in_round: false,
            in_round: false,
        };
        for &(node_idx, edge_idx) in node.iter() {
            let our_neighbor = &nodes[node_idx as usize];
            let the_edge = edges[edge_idx as usize];
            let me_in_their = our_neighbor
                .iter()
                .position(|&x| edge_idx == x.1)
                .unwrap()
                .try_into()
                .unwrap();
            actor.neighbors.push(Neighbor {
                snd: snds[node_idx as usize].clone(),
                me_in_their,
                cap: the_edge.cap,
                flow: 0,
            });
        }
        // eprintln!("neighs: {}", actor.neighbors.len());
        if i == 0 {
            // source, send messages to it's neighbors pushing them
            let mut total = 0;
            for neigh in actor.neighbors.iter() {
                total += neigh.cap;
                neigh
                    .snd
                    .send(Msg::PushToYou {
                        flow: neigh.cap,
                        me_in_your: neigh.me_in_their,
                    })
                    .await
                    .unwrap()
            }
            let end_actor = EndActor {
                receiver: actor.receiver,
                neighbors: actor.neighbors,
                excess_flow: -total,
                height: nodes.len() as _,
                notifyer: sourcer.0.clone(),
            };
            source_start_excess = end_actor.excess_flow;
            tokio::spawn(run_end_actor(end_actor));
        } else if i == nodes.len() - 1 {
            // target,
            let end_actor = EndActor {
                receiver: actor.receiver,
                neighbors: actor.neighbors,
                excess_flow: 0,
                height: 0,
                notifyer: targeter.0.clone(),
            };
            tokio::spawn(run_end_actor(end_actor));
        } else {
            tokio::spawn(run_my_actor(actor));
        }
    }
    let mut source_rcv = sourcer.1;
    let mut target_rcv = targeter.1;
    let mut source_at = source_start_excess;
    let mut target_at = 0;
    // println!("starts: source: {}, target: {}", source_at, target_at);
    while source_at + target_at != 0 {
        tokio::select! {
            Some(v) = source_rcv.recv() => {
                println!("got something");
                source_at = v;
            }
            Some(v) = target_rcv.recv() => {
                println!("got target: {}", v);
                target_at = v;
            }
        }
    }
    println!("f = {}", target_at);
}

type NewNodeId = u16;

enum Msg {
    AreYouShorter {
        my_height: Height,
        me_in_your: NewNodeId,
    },
    IAm {
        is_shorter: bool,
        me_in_your: NewNodeId,
    },
    PushToYou {
        flow: Flow,
        me_in_your: NewNodeId,
    },
}
struct Neighbor {
    snd: mpsc::Sender<Msg>,
    me_in_their: NewNodeId,
    cap: Cap,
    flow: Flow,
}
async fn run_end_actor(mut actor: EndActor) {
    while let Some(msg) = actor.receiver.recv().await {
        match msg {
            Msg::AreYouShorter {
                my_height,
                me_in_your,
            } => {
                let is_shorter = my_height > actor.height;
                // we now now our height hierarchy, send them back a response
                let them = &actor.neighbors[me_in_your as usize];
                them.snd
                    .send(Msg::IAm {
                        is_shorter,
                        me_in_your: them.me_in_their,
                    })
                    .await
                    .unwrap();
            }
            Msg::IAm {
                is_shorter,
                me_in_your,
            } => unreachable!("we wont ask anyone about height"),
            Msg::PushToYou { flow, me_in_your } => {
                actor.excess_flow += flow;
                // TODO notify someone that our excess flow was changed
                actor.notifyer.send(actor.excess_flow).await.unwrap();
            }
        }
    }
}

struct EndActor {
    receiver: mpsc::Receiver<Msg>,
    neighbors: Vec<Neighbor>,
    excess_flow: Flow,
    height: Height,
    notifyer: mpsc::Sender<Flow>,
}

struct Actor {
    receiver: mpsc::Receiver<Msg>,
    neighbors: Vec<Neighbor>,
    excess_flow: Flow,
    height: Height,

    responses: NewNodeId,
    pushed_to_anyone_in_round: bool,
    /// are we currently in round of waiting for all our neighbors to respond
    in_round: bool,
}
async fn run_my_actor(mut actor: Actor) {
    while let Some(msg) = actor.receiver.recv().await {
        macro_rules! ask_all_neighs {
            () => {{
                actor.in_round = true;
                actor.responses = 0;
                actor.pushed_to_anyone_in_round = false;
                for neigh in actor.neighbors.iter() {
                    neigh
                        .snd
                        .send(Msg::AreYouShorter {
                            my_height: actor.height,
                            me_in_your: neigh.me_in_their,
                        })
                        .await
                        .unwrap();
                }
            }};
        }
        match msg {
            Msg::AreYouShorter {
                my_height,
                me_in_your,
            } => {
                let is_shorter = my_height > actor.height;
                // we now now our height hierarchy, send them back a response
                let them = &actor.neighbors[me_in_your as usize];
                them.snd
                    .send(Msg::IAm {
                        is_shorter,
                        me_in_your: them.me_in_their,
                    })
                    .await
                    .unwrap();
            }
            Msg::IAm {
                is_shorter,
                me_in_your,
            } => {
                actor.responses += 1; // we have received one response!
                if is_shorter {
                    // they are definitely shorter than us
                    // so we will look on our edge, see how much we can send and send that much + modify the edge + tell them to increase their excess flow
                    let our_edge = &mut actor.neighbors[me_in_your as usize];
                    // we can only send the minimum of our excess_flow and the edge cap - already flow
                    let can_send = (our_edge.cap - our_edge.flow).min(actor.excess_flow);
                    if can_send > 0 {
                        actor.pushed_to_anyone_in_round = true; // we pushed!

                        actor.excess_flow -= can_send; // decrease our excess
                        our_edge.flow += can_send; // increase on the edge because flow is going OUT
                        our_edge
                            .snd
                            .send(Msg::PushToYou {
                                flow: can_send,
                                me_in_your: our_edge.me_in_their,
                            })
                            .await
                            .unwrap();
                    }
                }
                // check if we got all the responses
                if actor.responses as usize >= actor.neighbors.len() {
                    // println!("we have {} responses!", actor.responses);
                    // if they all declined (we never pushed to ANYONE) we have to relabel and ask everyone AGAIN
                    if actor.pushed_to_anyone_in_round == false {
                        actor.height += 1;
                        ask_all_neighs!();
                    } else {
                        // else, if we have any excess, we should ask all our neighbors again!
                        if actor.excess_flow > 0 {
                            ask_all_neighs!();
                        } else {
                            actor.in_round = false; // not in round anymore! Were done until we got some more excess
                        }
                    }
                }
            }
            Msg::PushToYou { flow, me_in_your } => {
                actor.excess_flow += flow;
                // they sent to us so...
                actor.neighbors[me_in_your as usize].flow -= flow;
                // we 100% have excess preflow now, so we should definitely ask all our neighs EXCEPT if we already asked them
                if !actor.in_round {
                    ask_all_neighs!();
                }
            }
        }
    }
}
/// if go from lower to higher index of node, positive flow means from lower to higher. Opposite if higher to lower.
#[derive(Debug, Clone, Copy)]
struct Edge {
    cap: Cap,
    flow: Flow,
}
type Graph = (Vec<Vec<(NodeIdx, EdgeIdx)>>, Vec<Edge>, usize, Vec<EdgeIdx>);
fn parse(file: String) -> Graph {
    let mut it = file
        .lines()
        .map(|l| l.split(' ').map(|v| v.parse::<usize>().unwrap()));
    let mut r = it.next().unwrap();
    let n = r.next().unwrap();
    let m = r.next().unwrap();
    let c = r.next().unwrap();

    let mut nodes: Vec<Vec<(NodeIdx, EdgeIdx)>> = vec![vec![]; n];
    let mut edges: Vec<Edge> = Vec::with_capacity(m);

    for i in 0..m {
        let mut r = it.next().unwrap();
        let from = r.next().unwrap();
        let to = r.next().unwrap();
        let cap = r.next().unwrap();
        nodes[from].push((to as NodeIdx, i as EdgeIdx));
        nodes[to].push((from as NodeIdx, i as EdgeIdx));

        edges.push(Edge {
            cap: cap as Cap,
            flow: 0,
        });
    }

    let plan = it.map(|mut v| v.next().unwrap() as EdgeIdx).collect();

    (nodes, edges, c, plan)
}
