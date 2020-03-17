use actix::prelude::*;
use krpc_mars::{RPCClient, StreamClient};

use crate::krpc::space_center;
use crate::strelka::streams::{StreamUpdate, Vector3};

/// Streamer periodically gets polls and fetches values from KRPC streams
pub struct Streamer {
    client: RPCClient,
    stream_client: StreamClient,
    
    ut: Option<krpc_mars::StreamHandle<f64>>,
    alt: Option<krpc_mars::StreamHandle<f64>>,
    direction: Option<krpc_mars::StreamHandle<(f64, f64, f64)>>,
}

impl Streamer {
    pub fn new(krpc_client: RPCClient, stream_client: StreamClient) -> Self {
        let mut streamer = Streamer{ 
            client:krpc_client.clone(),
            stream_client,
            ut: None, 
            alt: None,
            direction: None,
        };

        if let Err(e) = streamer.initialise_streams() {
            error!("Initialising streams failed: {}", e);
        }

        streamer
    }

    fn initialise_streams(&mut self) -> Result<(), failure::Error> {
        // TODO: Maybe refactor all these calls. Seems very likely to be repeated all over the codebase without some thought.
        let vessel = self.client.mk_call(&space_center::get_active_vessel())?;
        let bodies = self.client.mk_call(&space_center::get_bodies())?;
        let kerbin_ref = self.client.mk_call(&bodies.get("Kerbin").unwrap().get_reference_frame())?; // TODO: Temporary hard-code to Kerbin
        let flight = self.client.mk_call(&vessel.flight(&kerbin_ref))?;

        let ut_handle = self.client.mk_call(&space_center::get_ut().to_stream())?;
        let alt_handle = self.client.mk_call(&flight.get_mean_altitude().to_stream())?;
        let dir_handle = self.client.mk_call(&flight.get_direction().to_stream())?;

        self.ut = Some(ut_handle);
        self.alt = Some(alt_handle);
        self.direction = Some(dir_handle);

        Ok(())
    } 

    fn get_apoapsis_altitude(&self) -> Result<f64, failure::Error> {
        let vessel = self.client.mk_call(&space_center::get_active_vessel())?;
        let orbit = self.client.mk_call(&vessel.get_orbit())?;
        let apo_altitude = self.client.mk_call(&orbit.get_apoapsis_altitude())?;
 
        Ok(apo_altitude)
    }

    fn get_time_to_apoapsis(&self) -> Result<f64, failure::Error> {
        let vessel = self.client.mk_call(&space_center::get_active_vessel())?;
        let orbit = self.client.mk_call(&vessel.get_orbit())?;
        let apo_time = self.client.mk_call(&orbit.get_time_to_apoapsis())?;
 
        Ok(apo_time)
    }

    // Get the number of fuel-depleted engines in the current stage
    fn get_depleted_engine_count(&self) -> Result<i16, failure::Error> {
        let vessel = self.client.mk_call(&space_center::get_active_vessel())?;
        let control = self.client.mk_call(&vessel.get_control())?;
        let stage_no = self.client.mk_call(&control.get_current_stage())?;
        let parts = self.client.mk_call(&vessel.get_parts())?;
        let stage_parts = self.client.mk_call(&parts.in_stage(stage_no))?;

        let mut depleted_engines = vec!();
        for part in stage_parts {
            let engine = self.client.mk_call(&part.get_engine())?;
            let has_fuel = self.client.mk_call(&engine.get_has_fuel())?;
            if !has_fuel {
                depleted_engines.push(engine);
            }
        }

        Ok(depleted_engines.len() as i16)
    }
}

impl Actor for Streamer {
    type Context = Context<Self>;
}

pub struct StreamValues;

impl actix::Message for StreamValues {
    type Result = Vec<Box<StreamUpdate>>;
}

impl Handler<StreamValues> for Streamer {
    type Result = MessageResult<StreamValues>;

    fn handle(&mut self, _: StreamValues, _: &mut Context<Self>) -> Self::Result {
        let mut results: Vec<Box<StreamUpdate>> = Vec::new();

        let update_result = self.stream_client.recv_update();
        if let Ok(update) = update_result {
            // TODO: Probably abstract this out soon, will do for now
            // Universal time stream
            if let Some(handle) = self.ut {
                match update.get_result(&handle) {
                    Ok(ut) => { results.push(Box::new(StreamUpdate::UniversalTime(ut))); },
                    Err(e) => { trace!("Failed to get ut stream update: {}", e)}
                }
            }

            // Altitude stream
            if let Some(handle) = self.alt {
                match update.get_result(&handle) {
                    Ok(altitude) => { results.push(Box::new(StreamUpdate::Altitude(altitude))); },
                    Err(e) => { trace!("Failed to get altitude stream update: {}", e)}
                }
            }

            // Direction-based streams
            if let Some(handle) = self.direction {
                match update.get_result(&handle) {
                    Ok(raw_dir) => {
                        // Compute the pitch - the angle between the vessels direction and
                        // the direction in the horizon plane
                        let vessel_dir = Vector3::from(raw_dir);
                        let horizon = Vector3::new(0.0, vessel_dir.y(), vessel_dir.z());
                        let angle = vessel_dir.angle_between(&horizon);
                        let pitch = if vessel_dir.x() < 0.0 { -angle } else { angle };
                        results.push(Box::new(StreamUpdate::Pitch(pitch))); 
                    },
                    Err(e) => { trace!("Failed to get pitch stream update: {}", e)}
                }
            }

            // Apoapsis streams
            // KRPC does not like if we try to stream orbit values the same way as all the other values. After the first
            // successful request, all subsequent stream value requests fail. So we have to get orbit values manually!

            // Apoapsis altitude
            if let Ok(apo_alt) = self.get_apoapsis_altitude() {
                results.push(Box::new(StreamUpdate::Apoapsis(apo_alt)));
            }

            // Time to apoapsis
            if let Ok(apo_time) = self.get_time_to_apoapsis() {
                results.push(Box::new(StreamUpdate::TimeToApoapsis(apo_time)));
            }

            // Depleted engines count
            if let Ok(depleted_engines) = self.get_depleted_engine_count() {
                results.push(Box::new(StreamUpdate::EnginesDepleted(depleted_engines)));
            }
        }

        MessageResult(results)
    }
}
