// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Continuous BYTE joint animation, tessellated by egui and drawn by wgpu.
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use std::f32::consts::{PI, TAU};
const INK: Color32 = Color32::from_rgb(25, 40, 69);
// A distinct cream also lets the isolated GUI regression locate the cat in a screenshot.
pub const CREAM: Color32 = Color32::from_rgb(255, 235, 197);
const BLUE: Color32 = Color32::from_rgb(44, 132, 255);
pub const STRIDE: f32 = 92.0;

pub struct Pose { pub phase: f32, pub gait: f32, pub facing: f32, pub sleep: f32, pub time: f32 }

pub fn paint(painter: &egui::Painter, center: Pos2, pose: &Pose) {
    let gait = pose.gait;
    let bob = -(pose.phase * 2.0).cos() * 1.8 * gait + (pose.time * 1.8).sin() * 0.6 * (1.0 - gait);
    let point = |x: f32, y: f32| center + Vec2::new(x * pose.facing, y + bob);
    let ellipse = |at: Pos2, radii: Vec2, color: Color32| {
        painter.add(egui::Shape::ellipse_filled(at, radii + Vec2::splat(2.2), INK));
        painter.add(egui::Shape::ellipse_filled(at, radii, color));
    };
    let curve = |points: [Pos2; 4], width: f32, color: Color32| {
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, Stroke::new(width, color)));
    };
    // Tail swings continuously from its root, instead of jumping between atlas frames.
    let wag = (pose.phase + 0.8).sin() * 7.0 * gait + (pose.time * 2.1).sin() * 2.0;
    let tail = [point(-27.0, 8.0), point(-51.0, 11.0), point(-68.0, -17.0 + wag), point(-59.0, -27.0 + wag)];
    curve(tail, 13.0, INK); curve(tail, 8.2, CREAM);
    painter.circle_filled(tail[3], 6.5, INK); painter.circle_filled(tail[3], 4.1, CREAM);
    // Two bone legs with a smooth swing and planted stance. Diagonal pairs alternate.
    for (index, (hip_x, far)) in [(-24.0, true), (22.0, true), (-25.0, false), (23.0, false)].into_iter().enumerate() {
        let phase = pose.phase + if index == 0 || index == 3 { PI } else { 0.0 };
        let stride = phase.sin() * 14.0 * gait;
        let lift = phase.cos().max(0.0) * 11.0 * gait;
        let hip = point(hip_x, 15.0);
        let knee = point(hip_x + stride * 0.45 - 3.0, 28.0 - lift * 0.25);
        let foot = point(hip_x + stride + 3.0, 40.0 - lift - pose.sleep * 4.0);
        let color = if far { Color32::from_rgb(229, 203, 165) } else { CREAM };
        let leg = [hip, knee, knee + Vec2::new(1.0, 4.0), foot];
        curve(leg, 13.0, INK); curve(leg, 8.4, color);
        painter.circle_filled(foot, 7.0, INK); painter.circle_filled(foot, 4.8, color);
    }
    ellipse(point(-3.0, 9.0 + pose.sleep * 3.0), Vec2::new(34.0, 23.0), CREAM);
    // Bandana, ears and round head retain BYTE's navy / cream / blue identity.
    painter.add(egui::Shape::convex_polygon(vec![point(14.0, 3.0), point(43.0, 5.0), point(27.0, 30.0)], BLUE, Stroke::new(2.2, INK)));
    let symbol = [point(25.0, 12.0), point(29.0, 15.0), point(25.0, 18.0)];
    painter.line_segment([symbol[0], symbol[1]], Stroke::new(2.0, Color32::WHITE));
    painter.line_segment([symbol[1], symbol[2]], Stroke::new(2.0, Color32::WHITE));
    painter.line_segment([point(31.0, 19.0), point(36.0, 19.0)], Stroke::new(2.0, Color32::WHITE));
    let nod = (pose.phase * 2.0 + 0.4).sin() * gait * 1.2 + pose.sleep * 6.0;
    let head = |x: f32, y: f32| point(x, y + nod);
    for (left, tip, right) in [(4.0, 9.0, 23.0), (34.0, 46.0, 53.0)] {
        painter.add(egui::Shape::convex_polygon(vec![head(left, -27.0), head(tip, -49.0), head(right, -30.0)], INK, Stroke::new(2.0, INK)));
    }
    ellipse(head(29.0, -15.0), Vec2::new(29.0, 25.0), CREAM);
    let blink = ((pose.time * 0.26).fract() - 0.96).abs() < 0.017;
    let eye_open = (1.0 - pose.sleep) * if blink { 0.05 } else { 1.0 };
    for x in [19.0, 40.0] {
        if eye_open > 0.15 {
            ellipse(head(x, -18.0), Vec2::new(1.3, 2.0 * eye_open), INK);
        } else {
            curve([head(x - 3.0, -17.0), head(x - 1.0, -15.0), head(x + 1.0, -15.0), head(x + 3.0, -17.0)], 2.0, INK);
        }
    }
    for x in [9.0, 49.0] { painter.circle_filled(head(x, -6.0), 4.3, Color32::from_rgb(255, 166, 55)); }
    painter.circle_filled(head(30.0, -8.0), 1.8, INK);
    curve([head(24.0, -3.0), head(27.0, 0.0), head(29.0, -1.0), head(30.0, -4.0)], 1.6, INK);
    curve([head(30.0, -4.0), head(31.0, -1.0), head(33.0, 0.0), head(36.0, -3.0)], 1.6, INK);
    for sign in [-1.0, 1.0] {
        painter.line_segment([head(29.0 + sign * 27.0, -14.0), head(29.0 + sign * 35.0, -17.0)], Stroke::new(1.8, INK));
        painter.line_segment([head(29.0 + sign * 28.0, -8.0), head(29.0 + sign * 35.0, -7.0)], Stroke::new(1.8, INK));
    }
    if pose.sleep > 0.85 {
        // The little drifting z is vector geometry too: no font or texture upload per frame.
        let drift = (pose.time * 0.35).fract();
        let origin = point(48.0, -46.0 - drift * 6.0);
        let ink = INK.gamma_multiply((1.0 - drift) * pose.sleep);
        for (a, b) in [((0.0, 0.0), (6.0, 0.0)), ((6.0, 0.0), (0.0, 6.0)), ((0.0, 6.0), (6.0, 6.0))] {
            painter.line_segment([origin + Vec2::new(a.0, a.1), origin + Vec2::new(b.0, b.1)], Stroke::new(1.8, ink));
        }
    }
}

pub fn phase(distance: f32) -> f32 { (distance / STRIDE * TAU).rem_euclid(TAU) }
