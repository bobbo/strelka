extern crate krpc_mars;
extern crate failure;
extern crate actix;
extern crate actix_rt;

#[allow(dead_code)]
mod krpc;
mod strelka;

use strelka::launch_controller;

#[actix_rt::main]
async fn main() {

    match launch_controller::LaunchController::new() {
        Ok(mut ctl) => ctl.start_launch().await,
        Err(_) => panic!("Failed to start launch")
 
    };

}