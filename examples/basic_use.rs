use std::fmt::Display;
use std::thread;
use borrowing_from_session_types::{Snd, Recv, Return, send, split, recv, End};


fn forward_once<T: Display>(session: Recv<T, Snd<T, Return>>) {
    recv!(session, x);
    println!("Forwarding {x} in thread {:?}", thread::current().id());
    send!(session, x);
    drop(session);
}


fn main() {
    // Create channels
    let (tx, rx) = kanal::bounded::<u32>(0);

    // Create sessions
    let session1 = Snd::new(tx.clone(), Recv::new(rx.clone(), Snd::new(tx.clone(), End)));
    let session2 = Recv::new(rx.clone(), Snd::new(tx.clone(), Recv::new(rx.clone(), End)));

    let thread_one = thread::spawn(move || {
        send!(session1, 100);
        split!(session1 => borrow, session1);
        forward_once(borrow)
    });

    let thread_two = thread::spawn(move || {
        split!(session2 => borrow, session2);
        forward_once(borrow);
        recv!(session2, result);
        println!("{}", result);
    });


    thread_one.join().unwrap();
    thread_two.join().unwrap();
}
