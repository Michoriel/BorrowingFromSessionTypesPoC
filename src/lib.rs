use std::marker::PhantomData;
use kanal::{Receiver, Sender};

// Session Types
// (Send is a part of the stdlib prelude)
pub struct Snd<T, Cont>(Sender<T>, Cont, PanicOnDrop);
pub struct Recv<T, Cont>(Receiver<T>, Cont, PanicOnDrop);
pub struct End;
pub struct Return<'a>(PhantomData<&'a ()>);


// Actual usage API
impl<T, Cont> Snd<T, Cont> {
    pub fn new(sender: Sender<T>, cont: Cont) -> Self {
        Self(sender, cont, PanicOnDrop)
    }
    pub fn send(self, payload: T) -> Cont {
        self.0.send(payload).unwrap();
        self.2.disarm();
        self.1
    }
}


impl<T, Cont> Recv<T, Cont> {
    pub fn new(receiver: Receiver<T>, cont: Cont) -> Self {
        Self(receiver, cont, PanicOnDrop)
    }
    pub fn recv(self) -> (T, Cont) {
        let result = self.0.recv().unwrap();
        self.2.disarm();
        (result, self.1)
    }
}


// We have only affine types, rather than linear types. This means that a user could borrow some
// prefix, then drop it without using it. This would allow them to violate the protocol.
// Panicking in the drop implementation means we can detect this at runtime (although a compile time
// check would be preferred if it were possible)
struct PanicOnDrop;

impl PanicOnDrop {
    fn disarm(self) {
        std::mem::forget(self);
    }
}

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        // Do not panic if we are already panicking, this would trigger an immediate abort and could
        // obscure the original error
        if !std::thread::panicking() {
            panic!("Dropped a session before it was used")
        }
    }
}


// Split function
pub trait Split<Into> {
    type Remainder;

    // Arguably, this shouldn't be unsafe, because it shouldn't allow for memory/thread safety
    // violations, however we are attempting to extend the guarantees using session types, and this
    // does allow violating our new guarantees.
    unsafe fn split(self) -> (Into, Self::Remainder);
}


// End can only be split into Return, End
impl Split<Return<'static>> for End {
    type Remainder = End;

    unsafe fn split(self) -> (Return<'static>, Self::Remainder) {
        (Return(PhantomData), self)
    }
}


// Send<T, Cont> can be split into Send<T, P>, Remainder for any P, Remainder than Cont can
// be split into
impl<T, P, Cont> Split<Snd<T, P>> for Snd<T, Cont> where Cont: Split<P> {
    type Remainder = Cont::Remainder;

    unsafe fn split(self) -> (Snd<T, P>, Self::Remainder) {
        let (p, remainder) = unsafe{self.1.split()};
        (Snd(self.0, p, self.2), remainder)
    }
}

// Alternatively we could just take an "empty" (only return) session and leave the rest behind
impl<T, Cont> Split<Return<'static>> for Snd<T, Cont> {
    type Remainder = Self;

    unsafe fn split(self) -> (Return<'static>, Self::Remainder) {
        (Return(PhantomData), self)
    }
}


// Recv<T, Cont> can be split into Send<T, P>, Remainder for any P, Remainder than Cont can
// be split into
impl<T, P, Cont> Split<Recv<T, P>> for Recv<T, Cont> where Cont: Split<P> {
    type Remainder = Cont::Remainder;

    unsafe fn split(self) -> (Recv<T, P>, Self::Remainder) {
        let (p, remainder) = unsafe{self.1.split()};
        (Recv(self.0, p, self.2), remainder)
    }
}


// Alternatively we could just take an "empty" (only return) session and leave the rest behind
impl<T, Cont> Split<Return<'static>> for Recv<T, Cont> {
    type Remainder = Self;

    unsafe fn split(self) -> (Return<'static>, Self::Remainder) {
        (Return(PhantomData), self)
    }
}


// Lifetime attaching mechanism
// Reversed from first version, now this single trait is implemented for the final "restricted" type
// This is simpler, means that we don't need a second trait just to help convey type information,
// and means that borrowing errors don't accidentally also cause a type error (which obscures the
// borrow error)
pub trait Restricted<'a> {
    type Unrestricted;
    fn from_unrestricted<T>(unrestricted: Self::Unrestricted, _: &'a T) -> Self;
}


impl<'a> Restricted<'a> for Return<'a>
{
    type Unrestricted = Return<'static>;

    fn from_unrestricted<T>(unrestricted: Self::Unrestricted, _: &'a T) -> Self {
        Return(PhantomData)
    }
}


impl<'a, U, Cont> Restricted<'a> for Snd<U, Cont> where Cont: Restricted<'a> {
    type Unrestricted = Snd<U, Cont::Unrestricted>;

    fn from_unrestricted<T>(unrestricted: Self::Unrestricted, t: &'a T) -> Self {
        Self(unrestricted.0, Cont::from_unrestricted(unrestricted.1, t), unrestricted.2)
    }
}


impl<'a, U, Cont> Restricted<'a> for Recv<U, Cont> where Cont: Restricted<'a> {
    type Unrestricted = Recv<U, Cont::Unrestricted>;

    fn from_unrestricted<T>(unrestricted: Self::Unrestricted, t: &'a T) -> Self {
        Self(unrestricted.0, Cont::from_unrestricted(unrestricted.1, t), unrestricted.2)
    }
}


// End-user macros
#[macro_export]
macro_rules! split {
    ($original: ident => $a: ident, $b: ident) => {
        // Split session - Lifetimes are not yet applied
        let (unrestricted, $b) = unsafe {$crate::Split::split($original)};
        let $a = $crate::Restricted::from_unrestricted(unrestricted, &$b);
    }
}

#[macro_export]
macro_rules! send {
    ($session: ident, $payload: expr) => {
        let $session = $session.send($payload);
    }
}

#[macro_export]
macro_rules! recv {
    ($session: ident, $destination: ident) => {
        let ($destination, $session) = $session.recv();
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_test() {
        let p = PanicOnDrop;
        p.disarm();
    }
    #[test]
    #[should_panic]
    fn drop_test_2() {
        let _p = PanicOnDrop;
    }
}
