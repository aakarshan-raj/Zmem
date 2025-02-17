use colored::Colorize;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};

use crate::utils::{format_size, get_cmd, parse_value};
use crate::AnyError;

#[derive(Default, Clone, Copy)]
pub struct MemoryStats {
    pub total: u64,
    pub free: u64,
    pub available: u64,
    pub used: u64,
    pub shared: u64,
    pub buffers: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub zswap: u64,
    pub zswap_compressed: u64,
    pub swap_cached: u64,
    pub compression_ratio: f64,
    pub swap_used: u64,
    pub totalvmem: u64,
    pub freevmem: u64,
    pub usedvmem: u64,
    pub availablevmem: u64,
}

impl MemoryStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// ### Update
    /// Uses `/proc/meminfo` to get the memory stats
    pub fn update(&mut self) -> Result<(), AnyError> {
        let contents = fs::read_to_string("/proc/meminfo")?;
        for line in contents.lines() {
            // Split the line into key and value
            let mut split = line.split_whitespace();
            let (key, value) = (
                split.next().ok_or("bad file format")?,
                split.next().ok_or("bad file format")?,
            );
            match key {
                "MemTotal:" => self.total = value.parse()?,
                "MemFree:" => self.free = value.parse()?,
                "MemAvailable:" => self.available = value.parse()?,
                "MemUsed:" => self.used = value.parse()?,
                "Shmem:" => self.shared = value.parse()?,
                "Buffers:" => self.buffers = value.parse()?,
                "Cached:" => self.cached = value.parse()?,
                "SwapTotal:" => self.swap_total = value.parse()?,
                "SwapFree:" => self.swap_free = value.parse()?,
                "Zswap:" => self.zswap_compressed = value.parse()?,
                "Zswapped:" => self.zswap = value.parse()?,
                "SwapCached:" => self.swap_cached = value.parse()?,
                _ => (),
            }
        }
        self.used = self.total - self.free - self.buffers - self.cached;
        self.swap_used = self.swap_total - self.swap_free;
        self.compression_ratio = self.zswap as f64 / self.zswap_compressed as f64;
        self.totalvmem = self.total + self.swap_total;
        self.freevmem = self.free + self.swap_free;
        self.usedvmem = self.used + self.swap_used;
        self.availablevmem = self.available + self.swap_free;

        Ok(())
    }

    /// ### Display
    /// Displays the memory stats in a human readable format:
    /// ```
    ///             total            used            free          shared      buff/cache       available
    ///Mem:      7.15 GB         4.91 GB       340.04 MB       122.59 MB         1.91 GB         1.98 GB
    ///Swap:     9.77 GB         2.17 GB         7.60 GB                       256.40 MB         7.60 GB
    ///Total:   16.92 GB         7.08 GB         7.93 GB                                         9.58 GB
    ///
    ///            Zswap      Compressed           Ratio
    ///Zswap:    1.68 GB       764.75 MB           2.256
    /// ```
    pub fn display(&self) {
        let fmt = |s: u64| format!("{:>15}", format_size(s));
        let fmt_mem = |s: u64| format!("{:>12}", format_size(s));
        let fmt_swap = |s: u64| format!("{:>11}", format_size(s));
        let fmt_total = |s: u64| format!("{:>10}", format_size(s));
        let fmt_zswap = |s: u64| format!("{:>10}", format_size(s));
        let fmt_ratio = |s: f64| format!("{:>15.3}", s);

        println!(
            "{:>17} {:>15} {:>15} {:>15} {:>15} {:>15}",
            "total".bold(),
            "used".bold(),
            "free".bold(),
            "shared".bold(),
            "buff/cache".bold(),
            "available".bold()
        );
        println!(
            "{} {} {} {} {} {} {}",
            "Mem:".bold().cyan(),
            fmt_mem(self.total).green(),
            fmt(self.used).red(),
            fmt(self.free).cyan(),
            fmt(self.shared).yellow(),
            fmt(self.buffers + self.cached).magenta(),
            fmt(self.available).blue()
        );
        println!(
            "{} {} {} {} {:>15} {} {}",
            "Swap:".bold().purple(),
            fmt_swap(self.swap_total).green(),
            fmt(self.swap_used).red(),
            fmt(self.swap_free).cyan(),
            "",
            fmt(self.swap_cached).yellow(),
            fmt(self.swap_free).blue()
        );
        println!(
            "{} {} {} {} {:>15} {:>15} {}",
            "Total:".bold().blue(),
            fmt_total(self.totalvmem).green(),
            fmt(self.usedvmem).red(),
            fmt(self.freevmem).cyan(),
            "",
            "",
            fmt(self.availablevmem).blue()
        );
        println!(
            "\n{:>17} {:>15} {:>15}",
            "Zswap".bold(),
            "Compressed".bold(),
            "Ratio".bold()
        );
        println!(
            "{} {} {} {}",
            "Zswap:".bold().purple(),
            fmt_zswap(self.zswap).green(),
            fmt(self.zswap_compressed).red(),
            fmt_ratio(self.compression_ratio).cyan()
        );
    }
}

#[derive(Default, Clone)]
pub struct ProcessMemoryStats {
    pub pid: u32,
    pub username: String,
    pub command: String,
    pub swap: u64,
    pub uss: u64,
    pub pss: u64,
    pub rss: u64,
}

impl ProcessMemoryStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the process memory stats
    /// # Examples
    /// ```
    /// let mut pms = ProcessMemoryStats::new();
    /// pms.update(1)?;
    /// ```
    pub fn update(&mut self, pid: &u32) -> Result<(), AnyError> {
        self.command = get_cmd(*pid)?;
        if self.command.len() > 50 {
            self.command.truncate(50);
        }

        self.pid = *pid;

        // This is the sum of all the smaps data but it is much more performant to get it this way.
        // Since 4.14 and requires CONFIG_PROC_PAGE_MONITOR
        let smaps_file = File::open(format!("/proc/{}/smaps_rollup", pid))?;
        let mut reader = BufReader::new(smaps_file);
        let mut line = String::new(); // Line to be reused, saves allocations, after testing it seems to save 5-10% of the time

        // rss, pss, private_clean + private_dirty (uss), swap
        // Local variables are faster than struct fields, Data locality is important
        let mut mem_values = (0, 0, 0, 0);

        while reader.read_line(&mut line)? > 0 {
            match &line[..10] {
                // lines are hardcoded to be longer than 10 chars in the kernel code so this is "safe"
                "Rss:      " => mem_values.0 = parse_value(&line[5..])?,
                "Pss:      " => mem_values.1 = parse_value(&line[5..])?,
                "Private_Cl" | "Private_Di" => mem_values.2 += parse_value(&line[14..])?,
                "Swap:     " => mem_values.3 = parse_value(&line[6..])?,
                _ => (),
            }

            line.clear();
        }

        self.rss = mem_values.0;
        self.pss = mem_values.1;
        self.uss = mem_values.2;
        self.swap = mem_values.3;

        Ok(())
    }

    pub fn display(&self) {
        let fmt = |s: String| format!("{:>14}", s);

        println!(
            "{:>10} {} {} {} {} {}",
            self.pid,
            fmt(format_size(self.swap)).red(),
            fmt(format_size(self.uss)).green(),
            fmt(format_size(self.pss)).blue(),
            fmt(format_size(self.rss)).cyan(),
            self.command
        );
    }
}
