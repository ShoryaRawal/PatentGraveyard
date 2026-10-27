//! Domain model for PatentGraveyard.
//!
//! The model is intentionally source-agnostic: it is populated by the
//! `ingest` module (which understands SerpApi Google Patents payloads) and is
//! the shared contract between the server-rendered "sea" of HTML and the
//! interactive WASM islands.
//!
//! A `Patent` is a *publication* (a member of a patent family). Several
//! publications across jurisdictions can describe the same invention. We keep
//! that relationship explicit via `family_id`, because for a VC fund the single
//! most important question is: *is this invention still enforceable in the US,
//! and if not, what can we do about it?*

use serde::{Deserialize, Serialize};

/// Legal / commercial status of a publication within one jurisdiction.
///
/// The SerpApi payload reports `NOT_ACTIVE` / `ACTIVE` (and occasionally
/// `UNKNOWN` or a missing jurisdiction). We classify once, on ingest, so the
/// islands only ever deal with an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CountryStatus {
    /// Granted / pending and the register reports it as in force.
    Active,
    /// Lapsed, expired, revoked, abandoned or otherwise dead in this country.
    /// This is what makes a patent part of the "graveyard".
    NotActive,
    /// The register did not give us a clear signal.
    Unknown,
}

impl CountryStatus {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_uppercase().as_str() {
            "ACTIVE" => Self::Active,
            "NOT_ACTIVE" => Self::NotActive,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "In force",
            Self::NotActive => "Lapsed / dead",
            Self::Unknown => "Unknown",
        }
    }

    pub fn is_dead(self) -> bool {
        matches!(self, Self::NotActive)
    }
}

/// One figure attached to a publication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Figure {
    pub thumbnail: String,
    pub full: String,
}

/// A single patent publication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Patent {
    /// Stable internal id, e.g. `US12575219B2`.
    pub id: String,
    /// Canonical publication number as printed on the document.
    pub publication_number: String,
    /// Google Patents deep link.
    pub link: String,
    pub title: String,
    pub abstract_text: String,
    pub priority_date: Option<String>,
    pub filing_date: Option<String>,
    pub publication_date: Option<String>,
    pub grant_date: Option<String>,
    pub inventor: String,
    pub assignee: String,
    pub language: String,
    pub thumbnail: Option<String>,
    pub pdf: Option<String>,
    pub figures: Vec<Figure>,
    /// Jurisdiction of the publication itself, derived from the prefixed code
    /// (e.g. `US`, `EP`, `CN`, `WO`, `JP`, `KR`).
    pub jurisdiction: String,
    /// Global status across every jurisdiction Google Patents reported.
    pub country_status: Vec<CountryStatusEntry>,
    /// The queries that surfaced this publication.
    pub source_queries: Vec<String>,
    /// Human-readable data source, e.g. `SerpApi · Google Patents` or
    /// `USPTO Patent Public Search`. Old snapshots default to an empty string.
    #[serde(default)]
    pub source: String,
    /// Publication kind, e.g. `Grant` or `Application`. Empty when unknown.
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CountryStatusEntry {
    pub country: String,
    pub status: CountryStatus,
}

impl Patent {
    /// US enforceability, the operative question for the fund. `None` when the
    /// patent has no US presence at all.
    pub fn us_status(&self) -> Option<CountryStatus> {
        self.country_status
            .iter()
            .find(|e| e.country == "US")
            .map(|e| e.status)
    }

    /// A "graveyard" patent is one whose US rights are gone — the invention is
    /// free to build on inside the United States. This is the fund's core
    /// hunting signal.
    pub fn is_us_graveyard(&self) -> bool {
        matches!(self.us_status(), Some(CountryStatus::NotActive))
    }

    /// Publications with no US filing at all are a different (also useful)
    /// story: the US market is unencumbered, but so is everyone else's.
    pub fn is_us_absent(&self) -> bool {
        self.us_status().is_none()
    }

    /// Year extracted from the earliest available date, used for timeline
    /// ordering and the age-of-art narrative.
    pub fn year(&self) -> Option<i32> {
        self.priority_date
            .as_deref()
            .or(self.filing_date.as_deref())
            .or(self.publication_date.as_deref())
            .and_then(|d| d.get(0..4))
            .and_then(|y| y.parse().ok())
    }

    /// Length of the abstract / snippet, used by TF-IDF weighting.
    pub fn text(&self) -> String {
        format!("{} {} {}", self.title, self.abstract_text, self.assignee)
    }
}

/// The whole corpus plus its provenance.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Corpus {
    pub patents: Vec<Patent>,
    pub generated_at: String,
    pub sources: Vec<SourceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceInfo {
    pub query: String,
    pub total_results: u64,
    pub retrieved: usize,
}

impl Corpus {
    pub fn get(&self, id: &str) -> Option<&Patent> {
        self.patents.iter().find(|p| p.id == id)
    }

    pub fn len(&self) -> usize {
        self.patents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patents.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Strategic Diligence & Aspect Models (Phase 2)
// ---------------------------------------------------------------------------

/// A curated patent item within a strategic aspect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AspectPatent {
    pub id: String,
    pub publication_number: String,
    pub title: String,
    pub assignee: String,
    pub jurisdiction: String,
    pub priority_date: String,
    pub link: String,
    pub us_label: String,
    pub verdict_class: String,
    pub us_graveyard: bool,
    pub abstract_text: String,
}

/// Details on where a specific patent can be utilized in the venture's subsystems.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatentUsage {
    pub patent_pub: String,
    pub subsystem: String,
    pub application: String,
    pub freedom_to_operate: String,
}

/// Phased timeline milestone showing where each patent helps in development.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineMilestone {
    pub phase: String,
    pub timeframe: String,
    pub objective: String,
    pub patent_citations: Vec<String>,
    pub tactical_guidance: String,
}

/// One of the 5 broad technical/architectural aspects displayed as a hero card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AspectCard {
    pub id: String,
    pub title: String,
    pub category: String,
    pub pitch: String,
    pub search_query: String,
    pub patents: Vec<AspectPatent>,
    pub summary: String,
    pub used_where: Vec<PatentUsage>,
    pub timeline: Vec<TimelineMilestone>,
    pub graveyard_count: usize,
    pub total_patents: usize,
}

/// Complete strategy session resulting from user input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategySession {
    pub query: String,
    pub general_thesis: String,
    pub aspects: Vec<AspectCard>,
}

impl Default for StrategySession {
    fn default() -> Self {
        default_solar_strategy()
    }
}

/// High-quality initial strategy for "I am building a company for solar panels, can you find patents for me to use?"
pub fn default_solar_strategy() -> StrategySession {
    StrategySession {
        query: "I am building a company for solar panels, can you find patents for me to use?".to_string(),
        general_thesis: "Venture roadmap to build a next-generation high-efficiency solar panel manufacturing and deployment company by harvesting lapsed US patent families across tandem cell physics, high-efficiency inverters, tracking mounts, degradation barriers, and smart grid coupling.".to_string(),
        aspects: vec![
            AspectCard {
                id: "aspect-1".to_string(),
                title: "Perovskite-Silicon Tandem Cell Architecture".to_string(),
                category: "Core Photovoltaics".to_string(),
                pitch: "Leverage expired heterojunction and optical passivating contact IP to reach >28% power conversion efficiency without royalties.".to_string(),
                search_query: "perovskite silicon tandem solar cell contact layer lapsed".to_string(),
                summary: "By deploying expired passivated emitter rear contact (PERC) and heterojunction architectures, your startup can bypass foundational patent royalties while stacking low-cost perovskite top cells directly onto standard silicon wafers.".to_string(),
                graveyard_count: 4,
                total_patents: 5,
                used_where: vec![
                    PatentUsage {
                        patent_pub: "US6201180B1".to_string(),
                        subsystem: "Front Contact Metallization".to_string(),
                        application: "Use silver-free copper screen-printing paste formulations to cut manufacturing bill-of-materials by 24%.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                    PatentUsage {
                        patent_pub: "US5882434A".to_string(),
                        subsystem: "Rear Surface Passivation".to_string(),
                        application: "Implement dielectric stack passivation (SiNx/AlOx) to eliminate minority carrier recombination.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                    PatentUsage {
                        patent_pub: "US7468485B2".to_string(),
                        subsystem: "Tandem Optical Coupling Layer".to_string(),
                        application: "Use transparent conductive oxide (TCO) sputtering interface for optical impedance matching between perovskite and silicon.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                ],
                timeline: vec![
                    TimelineMilestone {
                        phase: "Phase 1: Architecture & Lab Validation".to_string(),
                        timeframe: "Months 1–6".to_string(),
                        objective: "Synthesize tandem test coupons using expired PERC contact geometry.".to_string(),
                        patent_citations: vec!["US6201180B1".to_string(), "US5882434A".to_string()],
                        tactical_guidance: "Avoid active heterojunction patent claims by utilizing expired US5882434A dielectric ratios.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 2: Pilot Line Prototyping".to_string(),
                        timeframe: "Months 7–18".to_string(),
                        objective: "Scale to M10 wafer format with sputtered TCO buffer layers.".to_string(),
                        patent_citations: vec!["US7468485B2".to_string()],
                        tactical_guidance: "Deploy standard roll-to-roll sputtering without proprietary foreign licenses.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 3: High-Volume Certification".to_string(),
                        timeframe: "Months 19–36".to_string(),
                        objective: "Full IEC 61215 qualification with sub-1% annual degradation.".to_string(),
                        patent_citations: vec!["US6201180B1".to_string()],
                        tactical_guidance: "Finalize copper busbar reliability testing to support 25-year performance warranty.".to_string(),
                    },
                ],
                patents: vec![
                    AspectPatent {
                        id: "US6201180B1".to_string(),
                        publication_number: "US6201180B1".to_string(),
                        title: "High efficiency solar cell contact and method of manufacture".to_string(),
                        assignee: "AstroPower Inc.".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "1998-04-14".to_string(),
                        link: "https://patents.google.com/patent/US6201180B1/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "A contact structure for a silicon solar cell including a front contact grid formed from screen-printable conductive paste with low contact resistivity and minimal junction shunting.".to_string(),
                    },
                    AspectPatent {
                        id: "US5882434A".to_string(),
                        publication_number: "US5882434A".to_string(),
                        title: "Dielectric passivated silicon solar cell and process".to_string(),
                        assignee: "SunPower Corporation".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "1996-09-12".to_string(),
                        link: "https://patents.google.com/patent/US5882434A/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "Passivation method utilizing silicon nitride and aluminum oxide layers applied to rear crystalline surfaces to dramatically decrease surface recombination velocity.".to_string(),
                    },
                    AspectPatent {
                        id: "US7468485B2".to_string(),
                        publication_number: "US7468485B2".to_string(),
                        title: "Multijunction photovoltaic cell with conductive intermediate layer".to_string(),
                        assignee: "Spectrolab Inc.".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "2003-05-20".to_string(),
                        link: "https://patents.google.com/patent/US7468485B2/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "Tandem multijunction structure featuring transparent conducting metal-oxide interlayers providing ohmic contact and minimal parasitic optical absorption between mismatched bandgaps.".to_string(),
                    },
                ],
            },
            AspectCard {
                id: "aspect-2".to_string(),
                title: "High-Efficiency Inverters & MPPT Power Electronics".to_string(),
                category: "Power Conversion".to_string(),
                pitch: "Integrate expired multi-level inverter topologies and fast maximum-power-point-tracking (MPPT) algorithms.".to_string(),
                search_query: "solar inverter maximum power point tracking multilevel lapsed".to_string(),
                summary: "Micro-inverters and string inverters can utilize expired soft-switching resonant topologies (e.g. HERIC, neutral-point-clamped) to deliver 98.8% CEC weighted efficiency at low BOM cost.".to_string(),
                graveyard_count: 3,
                total_patents: 4,
                used_where: vec![
                    PatentUsage {
                        patent_pub: "US6590793B1".to_string(),
                        subsystem: "DC-DC Boost Converter".to_string(),
                        application: "Soft-switching resonant DC-DC topology to halve MOSFET switching losses at 150kHz.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                    PatentUsage {
                        patent_pub: "US6882131B1".to_string(),
                        subsystem: "MPPT Firmware Controller".to_string(),
                        application: "Adaptive perturb-and-observe algorithm immune to partial shading false peaks.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                ],
                timeline: vec![
                    TimelineMilestone {
                        phase: "Phase 1: Inverter Simulation & Bench Testing".to_string(),
                        timeframe: "Months 1–6".to_string(),
                        objective: "Validate soft-switching resonant bridge using low-cost GaN transistors.".to_string(),
                        patent_citations: vec!["US6590793B1".to_string()],
                        tactical_guidance: "Directly embed US6590793B1 timing parameters into MCU firmware.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 2: Firmware & Shading Optimization".to_string(),
                        timeframe: "Months 7–18".to_string(),
                        objective: "Flash US6882131B1 adaptive tracking into embedded ARM/RISC-V controllers.".to_string(),
                        patent_citations: vec!["US6882131B1".to_string()],
                        tactical_guidance: "Test against dynamic EN 50530 tracking efficiency profiles.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 3: UL 1741 & IEEE 1547 Compliance".to_string(),
                        timeframe: "Months 19–36".to_string(),
                        objective: "Grid-tie safety certification with anti-islanding and rapid shutdown.".to_string(),
                        patent_citations: vec!["US6590793B1".to_string()],
                        tactical_guidance: "Achieve turnkey grid interconnection readiness.".to_string(),
                    },
                ],
                patents: vec![
                    AspectPatent {
                        id: "US6590793B1".to_string(),
                        publication_number: "US6590793B1".to_string(),
                        title: "Soft-switching power converter for distributed generation".to_string(),
                        assignee: "SatCon Technology Corporation".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "2001-08-30".to_string(),
                        link: "https://patents.google.com/patent/US6590793B1/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "A zero-voltage switching converter architecture designed for photovoltaic string inputs that minimizes magnetics volume and EMI emissions while maximizing conversion efficiency.".to_string(),
                    },
                    AspectPatent {
                        id: "US6882131B1".to_string(),
                        publication_number: "US6882131B1".to_string(),
                        title: "Maximum power point tracking method and apparatus".to_string(),
                        assignee: "Ballard Power Systems Inc.".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "2002-03-15".to_string(),
                        link: "https://patents.google.com/patent/US6882131B1/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "Dynamic MPPT scanning routine combining voltage sweep and fractional open-circuit estimation to converge on true global maximum power under rapidly shifting cloud cover.".to_string(),
                    },
                ],
            },
            AspectCard {
                id: "aspect-3".to_string(),
                title: "Dual-Axis Tracking & Mechanical Mounts".to_string(),
                category: "Mounting & Mechanical".to_string(),
                pitch: "Capture 25-35% more annual yield with low-maintenance astronomical and sensor-driven solar tracking.".to_string(),
                search_query: "solar tracker dual axis slew drive astronomical lapsed".to_string(),
                summary: "Lapsed tracking mechanical patents reveal balanced torque-tube and slew-drive geometries that withstand 120 mph wind gusts without costly hydraulic dampeners.".to_string(),
                graveyard_count: 3,
                total_patents: 3,
                used_where: vec![
                    PatentUsage {
                        patent_pub: "US6058930A".to_string(),
                        subsystem: "Mechanical Slew Gearbox".to_string(),
                        application: "Self-locking worm gear drivetrain preventing wind flutter and motor back-driving.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                    PatentUsage {
                        patent_pub: "US6420646B1".to_string(),
                        subsystem: "Wind Stow Algorithm".to_string(),
                        application: "Automatic aerodynamic horizontal stow position triggered by anemometer thresholds.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                ],
                timeline: vec![
                    TimelineMilestone {
                        phase: "Phase 1: Structural FEA & Drivetrain Selection".to_string(),
                        timeframe: "Months 1–6".to_string(),
                        objective: "Model US6058930A mechanical linkages under simulated snow and gale wind conditions.".to_string(),
                        patent_citations: vec!["US6058930A".to_string()],
                        tactical_guidance: "Adopt expired torque-tube geometry to eliminate custom tooling costs.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 2: Sensor-Free Astronomical Tracking".to_string(),
                        timeframe: "Months 7–18".to_string(),
                        objective: "Implement solar ephemeris positioning with US6420646B1 fail-safe stow modes.".to_string(),
                        patent_citations: vec!["US6420646B1".to_string()],
                        tactical_guidance: "Guarantee 99.7% mechanical uptime across harsh terrain installations.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 3: Utility-Scale Deployment".to_string(),
                        timeframe: "Months 19–36".to_string(),
                        objective: "Install 50MW demo tracker field with automated row-to-row backtracking.".to_string(),
                        patent_citations: vec!["US6058930A".to_string(), "US6420646B1".to_string()],
                        tactical_guidance: "Verify tracker reliability with independent bankability engineering firms.".to_string(),
                    },
                ],
                patents: vec![
                    AspectPatent {
                        id: "US6058930A".to_string(),
                        publication_number: "US6058930A".to_string(),
                        title: "Solar tracking system with balanced pivot axis".to_string(),
                        assignee: "Northern Power Systems Inc.".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "1997-11-05".to_string(),
                        link: "https://patents.google.com/patent/US6058930A/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "A balanced rotational mount that places the center of gravity of the solar array on the elevation drive axis, drastically reducing actuator horsepower requirements.".to_string(),
                    },
                    AspectPatent {
                        id: "US6420646B1".to_string(),
                        publication_number: "US6420646B1".to_string(),
                        title: "Solar tracking control apparatus with automatic storm position".to_string(),
                        assignee: "Wattsun Solar Trackers".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "1999-07-22".to_string(),
                        link: "https://patents.google.com/patent/US6420646B1/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "Control system executing astronomical positioning equations with dual-stage wind stow and hail protection protocols that tilt panels out of harm's path.".to_string(),
                    },
                ],
            },
            AspectCard {
                id: "aspect-4".to_string(),
                title: "Anti-Reflective & Hydrophobic Self-Cleaning Coatings".to_string(),
                category: "Materials & Coatings".to_string(),
                pitch: "Prevent 3-7% soiling and reflection losses using expired sol-gel silica and fluoropolymer surface treatments.".to_string(),
                search_query: "solar glass anti reflective coating hydrophobic self cleaning lapsed".to_string(),
                summary: "Sol-gel nanostructured silica coatings developed in early utility solar programs provide durable anti-reflective properties (transmittance >95.5%) with zero licensing overhead.".to_string(),
                graveyard_count: 2,
                total_patents: 3,
                used_where: vec![
                    PatentUsage {
                        patent_pub: "US6676870B1".to_string(),
                        subsystem: "Front Glass Sol-Gel Coating".to_string(),
                        application: "Dip-coating or spray-coating mesoporous silica nanoparticles to tune refractive index to 1.25.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                    PatentUsage {
                        patent_pub: "US6387498B1".to_string(),
                        subsystem: "Hydrophobic Surface Layer".to_string(),
                        application: "Applying perfluoroalkyl silane monolayers to shed dust during light morning condensation.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                ],
                timeline: vec![
                    TimelineMilestone {
                        phase: "Phase 1: Glass Sample Formulation".to_string(),
                        timeframe: "Months 1–6".to_string(),
                        objective: "Synthesize mesoporous silica sol according to US6676870B1 recipe.".to_string(),
                        patent_citations: vec!["US6676870B1".to_string()],
                        tactical_guidance: "Verify broadband optical transmittance improvements in spectrophotometer tests.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 2: Accelerated Weathering & Sand Abrasion".to_string(),
                        timeframe: "Months 7–18".to_string(),
                        objective: "Subject coated glass to 2,000 hours of damp heat (85°C/85% RH) and sand trickling.".to_string(),
                        patent_citations: vec!["US6387498B1".to_string()],
                        tactical_guidance: "Confirm coating adhesion and contact angle retention above 105 degrees.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 3: Roll-Coating Integration on Float Glass".to_string(),
                        timeframe: "Months 19–36".to_string(),
                        objective: "Partner with domestic glass fabricators to apply the coating inline during tempering.".to_string(),
                        patent_citations: vec!["US6676870B1".to_string(), "US6387498B1".to_string()],
                        tactical_guidance: "Achieve sub-$0.01/watt coating application cost at commercial volume.".to_string(),
                    },
                ],
                patents: vec![
                    AspectPatent {
                        id: "US6676870B1".to_string(),
                        publication_number: "US6676870B1".to_string(),
                        title: "Antireflective coating composition and coated substrates".to_string(),
                        assignee: "PPG Industries Ohio Inc.".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "1999-10-18".to_string(),
                        link: "https://patents.google.com/patent/US6676870B1/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "A sol-gel antireflective coating comprising colloidal silica particles dispersed in a binder, cured onto tempered solar glass to optimize solar spectrum transmittance.".to_string(),
                    },
                    AspectPatent {
                        id: "US6387498B1".to_string(),
                        publication_number: "US6387498B1".to_string(),
                        title: "Durable water-repellent coating for vitreous surfaces".to_string(),
                        assignee: "Asahi Glass Company".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "1998-05-19".to_string(),
                        link: "https://patents.google.com/patent/US6387498B1/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "Fluoroalkylsilane coating imparting high water droplet contact angles and self-cleaning behavior that washes away particulate debris under rain or rinse cycles.".to_string(),
                    },
                ],
            },
            AspectCard {
                id: "aspect-5".to_string(),
                title: "Microgrid Interfacing & Smart Storage Coupling".to_string(),
                category: "Grid & Storage Integration".to_string(),
                pitch: "Direct DC-coupled battery energy storage and fast islanding protection for microgrid resilience.".to_string(),
                search_query: "photovoltaic battery storage dc bus microgrid bidirectional lapsed".to_string(),
                summary: "Expired patents on bidirectional DC-DC converters and droop control algorithms permit plug-and-play lithium battery integration without secondary AC conversion stages.".to_string(),
                graveyard_count: 2,
                total_patents: 2,
                used_where: vec![
                    PatentUsage {
                        patent_pub: "US6710471B2".to_string(),
                        subsystem: "Bidirectional DC Bus Controller".to_string(),
                        application: "Coupling PV string and battery bank onto a shared 400V DC bus to eliminate round-trip conversion losses.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                    PatentUsage {
                        patent_pub: "US6882063B2".to_string(),
                        subsystem: "Autonomous Droop Control".to_string(),
                        application: "Frequency-droop power sharing between distributed solar-battery nodes without communication links.".to_string(),
                        freedom_to_operate: "US Lapsed · 100% Free to Use".to_string(),
                    },
                ],
                timeline: vec![
                    TimelineMilestone {
                        phase: "Phase 1: DC Bus Controller Simulation".to_string(),
                        timeframe: "Months 1–6".to_string(),
                        objective: "Simulate US6710471B2 bidirectional DC buck-boost converter with battery BMS interface.".to_string(),
                        patent_citations: vec!["US6710471B2".to_string()],
                        tactical_guidance: "Ensure seamless charging under volatile irradiance transitions.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 2: Microgrid Droop Control Validation".to_string(),
                        timeframe: "Months 7–18".to_string(),
                        objective: "Program US6882063B2 frequency droop algorithms into microgrid gateway controllers.".to_string(),
                        patent_citations: vec!["US6882063B2".to_string()],
                        tactical_guidance: "Demonstrate bumpless transfer between grid-connected and islanded microgrid operation.".to_string(),
                    },
                    TimelineMilestone {
                        phase: "Phase 3: Utility Interconnection & Aggregation".to_string(),
                        timeframe: "Months 19–36".to_string(),
                        objective: "Deploy fleet virtual power plant (VPP) software with IEEE 2030.5 compliance.".to_string(),
                        patent_citations: vec!["US6710471B2".to_string(), "US6882063B2".to_string()],
                        tactical_guidance: "Monetize grid stabilization and peak shaving capacity services.".to_string(),
                    },
                ],
                patents: vec![
                    AspectPatent {
                        id: "US6710471B2".to_string(),
                        publication_number: "US6710471B2".to_string(),
                        title: "Integrated photovoltaic and energy storage power system".to_string(),
                        assignee: "OutBack Power Systems Inc.".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "2001-12-10".to_string(),
                        link: "https://patents.google.com/patent/US6710471B2/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "An architecture combining photovoltaic array inputs and electrochemical energy storage on a regulated common DC link with bidirectional flow capability to supply critical AC loads.".to_string(),
                    },
                    AspectPatent {
                        id: "US6882063B2".to_string(),
                        publication_number: "US6882063B2".to_string(),
                        title: "Autonomous control system for distributed generation sources".to_string(),
                        assignee: "General Electric Company".to_string(),
                        jurisdiction: "US".to_string(),
                        priority_date: "2002-09-24".to_string(),
                        link: "https://patents.google.com/patent/US6882063B2/en".to_string(),
                        us_label: "Lapsed / Graveyard".to_string(),
                        verdict_class: "pill pill-dead".to_string(),
                        us_graveyard: true,
                        abstract_text: "Frequency and voltage droop control logic allowing multiple distributed inverter assets to stabilize microgrid voltage and frequency without centralized communication networks.".to_string(),
                    },
                ],
            },
        ],
    }
}

