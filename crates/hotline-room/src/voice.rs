//! What voice keeps in the room and reads from it, below the calls that use
//! it: the owner's voice settings, the ledger of what voice and teammates
//! have spent and the budget that gates it, and how a teammate's reply on a
//! call is split into what is said and what is shown.

pub mod ledger;
pub mod metering;
pub mod settings;
pub mod spoken;
