//! Audioscrobbler scrobbling for Last.fm, Libre.fm, and ListenBrainz,
//! plus shared play-threshold logic.

pub mod auth;
pub mod lastfm;
pub mod listenbrainz;
pub mod threshold;
pub mod track;

pub use auth::{AuthClient, AuthSession};
pub use lastfm::{AudioscrobblerClient, ScrobbleClient, ScrobbleService};
pub use listenbrainz::ListenBrainzClient;
pub use threshold::{
    audioscrobbler_eligible, play_threshold, AudioscrobblerRules, ListenThreshold,
};
pub use track::TrackInfo;
