//! Generic PID controller. Used to hold pressure at setpoint by adjusting
//! the VFD frequency — see `state_machine` and CLAUDE.md's "Capacity
//! control strategy".

pub struct Pid {
    kp: f32,
    ki: f32,
    kd: f32,
    setpoint: f32,
    output_min: f32,
    output_max: f32,
    integral: f32,
    prev_measurement: Option<f32>,
}

impl Pid {
    pub fn new(kp: f32, ki: f32, kd: f32, setpoint: f32, output_min: f32, output_max: f32) -> Self {
        Self {
            kp,
            ki,
            kd,
            setpoint,
            output_min,
            output_max,
            integral: 0.0,
            prev_measurement: None,
        }
    }

    /// Re-clamp the output range without resetting integral/derivative
    /// state — used when the VFD's live-read min/max frequency narrows
    /// the range after startup.
    pub fn set_output_limits(&mut self, min: f32, max: f32) {
        self.output_min = min;
        self.output_max = max;
        self.integral = self.integral.clamp(min, max);
    }

    /// Move the setpoint (operator +/- on the HMI). Integral is kept: the
    /// accumulated term reflects plant demand, not the old target, and
    /// derivative-on-measurement means no kick from the step either.
    pub fn set_setpoint(&mut self, setpoint: f32) {
        self.setpoint = setpoint;
    }

    /// Advance the controller by `dt_s` seconds given the latest
    /// `measurement`, returning the clamped output.
    ///
    /// Derivative is computed on measurement (not error) to avoid
    /// "derivative kick" from setpoint changes; integral uses
    /// clamped-output anti-windup (skips accumulating further in the
    /// direction that's already saturating the output).
    pub fn update(&mut self, measurement: f32, dt_s: f32) -> f32 {
        let error = self.setpoint - measurement;

        let d_term = match self.prev_measurement {
            Some(prev) if dt_s > 0.0 => -self.kd * (measurement - prev) / dt_s,
            _ => 0.0,
        };
        self.prev_measurement = Some(measurement);

        let p_term = self.kp * error;
        let unsaturated = p_term + self.ki * self.integral + d_term;

        let saturated_high = unsaturated >= self.output_max && error > 0.0;
        let saturated_low = unsaturated <= self.output_min && error < 0.0;
        if !saturated_high && !saturated_low {
            self.integral += error * dt_s;
        }

        (p_term + self.ki * self.integral + d_term).clamp(self.output_min, self.output_max)
    }

    pub fn reset(&mut self) {
        self.integral = 0.0;
        self.prev_measurement = None;
    }
}
