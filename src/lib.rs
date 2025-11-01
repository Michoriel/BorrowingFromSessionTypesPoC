use std::marker::PhantomData;
use kanal::{Receiver, Sender};

// Session Types
// (Send is a part of the stdlib prelude)
pub struct Snd<T, Cont>(Sender<T>, Cont, PanicOnDrop);
pub struct Recv<T, Cont>(Receiver<T>, Cont, PanicOnDrop);
pub struct End;
pub struct Return<'a>(PhantomData<&'a ()>);
pub struct Recursion<Body>(Body);


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
pub trait RestrictLifetime<'a> {
    type Restricted: 'a;
    fn restrict_to_ref<T>(self, reference: &'a T) -> Self::Restricted;
}


// TODO: explain
impl<'a> RestrictLifetime<'a> for Return<'a> {
    type Restricted = Return<'a>;

    fn restrict_to_ref<T>(self, _: &'a T) -> Self::Restricted {
        Return(PhantomData)
    }
}

// Send
impl<'a, U: 'a, Cont> RestrictLifetime<'a> for Snd<U, Cont> where Cont: RestrictLifetime<'a> {
    type Restricted = Snd<U, Cont::Restricted>;

    fn restrict_to_ref<T>(self, reference: &'a T) -> Self::Restricted {
        Snd(self.0, self.1.restrict_to_ref(reference), self.2)
    }
}


// Recv
impl<'a, U: 'a, Cont> RestrictLifetime<'a> for Recv<U, Cont> where Cont: RestrictLifetime<'a> {
    type Restricted = Recv<U, Cont::Restricted>;

    fn restrict_to_ref<T>(self, reference: &'a T) -> Self::Restricted {
        Recv(self.0, self.1.restrict_to_ref(reference), self.2)
    }
}


// In order to aid the type solver, we need to explain how to find the unrestricted ('static)
// version of a borrowed session. If we do not, the trait solver will (correctly) identify that
// multiple different types could provide the same restricted type (of course this wouldn't make
// sense with what we want to do, but the trait solver doesn't know that)
pub trait Restricted<'a> {
    type Unrestricted: RestrictLifetime<'a, Restricted=Self>;
}

impl<'a> Restricted<'a> for Return<'a> {
    type Unrestricted = Return<'a>;
}
impl<'a, T, Cont> Restricted<'a> for Snd<T, Cont> where Cont: Restricted<'a>, T: 'a {
    type Unrestricted = Snd<T, Cont::Unrestricted>;
}
impl<'a, T, Cont> Restricted<'a> for Recv<T, Cont> where Cont: Restricted<'a>, T: 'a {
    type Unrestricted = Recv<T, Cont::Unrestricted>;
}


// Just calls into the RestrictLifetime::restrict_to_ref method, but the extra bounds ensure that
// it is unambiguous where the implementation is coming from
// (it might also be possible to avoid needing this by re-working RestrictLifetime...)
pub fn restrict_with_type_info<'a, Dest: Restricted<'a>, T>(unrestricted: Dest::Unrestricted, reference: &'a T) -> Dest {
    unrestricted.restrict_to_ref(reference)
}


// End-user macros
#[macro_export]
macro_rules! split {
    ($what: ident => $a: ident, $b: ident) => {
        // Split session - Lifetimes are not yet applied
        let (unrestricted, $b) = unsafe {$crate::Split::split($what)};
        let $a = $crate::restrict_with_type_info(unrestricted, &$b);
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
