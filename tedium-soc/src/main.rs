#![no_std]
#![no_main]

extern crate panic_halt;

use embedded_hal::prelude::*;
// use riscv::register;
use riscv_rt::entry;

use xrt86vx38_pac::{self, device::{self, Result, DeviceAccess, RegisterAddress, RegisterValue, Xyz, Channel, Timeslot}};
use xrt86vx38_pac::register::*;

struct Access {
    p: u32,
}

impl Access {
    fn new(p: u32) -> Self {
        Self {
            p,
        }
    }
}

impl DeviceAccess for Access {
    fn read(&self, address: RegisterAddress) -> xrt86vx38_pac::device::Result<RegisterValue> {
        unsafe {
            let p = self.p as *const u32;
            let p = p.offset(address as isize);
            let v = p.read_volatile();
            Ok(v as RegisterValue)
        }
    }

    fn write(&self, address: RegisterAddress, value: RegisterValue) -> device::Result<()> {
        unsafe {
            let p = self.p as *mut u32;
            let p = p.offset(address as isize);
            p.write_volatile(value as u32);
            Ok(())
        }
    }
}

type Device = xrt86vx38_pac::device::Device<Access>;

struct TestPoints {
    p: u32,
    v: u32,
}

impl TestPoints {
    fn new(p: u32) -> Self {
        Self {
            p,
            v: 0,
        }
    }

    fn toggle(&mut self, n: usize) {
        self.v ^= 1 << n;
        self.set_value(self.v);
    }

    fn set(&mut self, n: usize) {
        self.v |= 1 << n;
        self.set_value(self.v);
    }

    fn clear(&mut self, n: usize) {
        self.v &= !(1 << n);
        self.set_value(self.v);
    }

    fn set_value(&self, value: u32) {
        unsafe {
            let p = self.p as *mut u32;
            let p = p.offset(0);
            p.write_volatile(value);
        }
    }
}

struct FramerControl {
    p: u32
}

impl FramerControl {
    fn new(p: u32) -> Self {
        Self {
            p,
        }
    }

    fn set_reset(&self, value: bool) {
        unsafe {
            let p = self.p as *mut u32;
            let p = p.offset(0);
            p.write_volatile(value as u32);
        }
    }

    fn set_outputs_control(&self, value: bool) {
        unsafe {
            let p = self.p as *mut u32;
            let p = p.offset(1);
            p.write_volatile(value as u32);
        }
    }
}

struct Uart {
    p: u32,
}

impl Uart {
    fn new(p: u32) -> Self {
        Self {
            p,
        }
    }
    
    fn register_write(&self, n: u32, v: u32) {
        unsafe {
            let p = (self.p + n * 4) as *mut u32;
            p.write_volatile(v);
        }
    }

    fn register_read(&self, n: u32) -> u32 {
        unsafe {
            let p = (self.p + n * 4) as *mut u32;
            p.read_volatile()
        }
    }

    fn tx_data(&self, v: u32) {
        self.register_write(4, v);
    }

    fn tx_rdy(&self) -> bool {
        self.register_read(5) != 0
    }

    fn write_char(&self, c: u8) {
        while !self.tx_rdy() {}
        self.tx_data(c as u32);
    }
}

fn configure_channel<D: Xyz>(channel: &Channel<D>) -> Result<()> {
    // THEORY?
    // NOTE: I *think* the clock loss detection feature is not effective
    // in our case, as channels are currently configured to use TxSERCLK_n
    // as their transmit clock source ("External Timing Modee"). The FPGA
    // is "wired" to take the recovered clock from one of the channels and
    // mirror it to the TxSERCLK on all channels.

    channel.csr().write(|w| w
        .with_LCV_Insert(0)
        .with_Set_T1_Mode(1)
        .with_Sync_All_Transmitters_to_8kHz(0)
        .with_Clock_Loss_Detect(1)
        .with_CSS(ClockSource::External)
    )?;

    channel.licr().write(|w| w
        .with_FORCE_LOS(0)
        .with_Single_Rail_Mode(0)
        .with_LB(FramerLoopback::No)
        .with_Encode_B8ZS(0)
        .with_Decode_AMI_B8ZS(0)
    )?;

    channel.fsr().write(|w| w
        .with_Signaling_update_on_Superframe_Boundaries(1)
        .with_Force_CRC_Errors(0)
        .with_J1_MODE(0)
        .with_ONEONLY(1)    // Not the default, maybe more reliable sync?
        .with_FASTSYNC(0)
        .with_FSI(T1Framing::ExtendedSuperFrame)
    )?;

    channel.smr().write(|w| w
        .with_MFRAMEALIGN(0)    // Not used in base rate mode
        .with_MSYNC(0)    // Not used in base rate mode
        .with_Transmit_Frame_Sync_Select(0)
        .with_CRC6_Bits_Source_Select(0)
        .with_Framing_Bits_Source_Select(0)
    )?;

    channel.fcr().write(|w| w
        .with_Reframe(0)
        .with_Framing_with_CRC_Checking(1)
        .with_LOF_Tolerance(2)
        .with_LOF_Range(5)
    )?;

    // HDLC1 (for Facilities Data Link, right?)
    // Use "MOS" mode if we want 0b01111110 idle code with HDLC messages
    // (including reporting). Setting this makes the Adit 600s very happy,
    // stops the Adit from getting stuck when bringing up a channel.
    // Still gets stuck in payload loopback mode. Maybe it's important to
    // have MOS set *before* the channel starts sending frames, so that the
    // Adit doesn't autodetect(?) a BOS DLC channel instead of a MOS one?
    // So I've moved this to configure_channel().
    channel.dlcr1().modify(|m| m
        .with_SLC96_Data_Link_Enable(0)
        .with_MOS_ABORT_Disable(0)
        .with_Rx_FCS_DIS(0)
        .with_AutoRx(0)
        .with_Tx_ABORT(0)
        .with_Tx_IDLE(0)
        .with_Tx_FCS_EN(0)
        .with_MOS_BOSn(1)       
    )?;

    if true {
        // Performance reports
        channel.tsprmcr().modify(|m| m
            .with_FC_Bit(0)
            .with_PA_Bit(0)
            .with_U1_Bit(0)
            .with_U2_Bit(0)
            .with_R_Bit(0b0000)
        )?;
        channel.prcr().modify(|m| m
            .with_LBO_ADJ_ENB(0)
            .with_FAR_END(0)
            .with_NPRM(0b00)
            .with_C_R_Bit(0)
            .with_APCR(AutomaticPerformanceReport::EverySecond)
        )?;
    }

    channel.sbcr().write(|w| w
        .with_TxSB_ISFIFO(0)
        .with_SB_FORCESF(0)
        .with_SB_SFENB(0)
        .with_SB_SDIR(1)
        .with_SB_ENB(ReceiveSlipBuffer::SlipBuffer)
    )?;

    channel.ticr().write(|w| w
        .with_TxSyncFrD(0)
        .with_TxPLClkEnb_TxSync_Is_Low(0)
        .with_TxFr1544(0)
        .with_TxICLKINV(0)
        .with_TxIMODE(0b00)
    )?;

    channel.ricr().write(|w| w
        .with_RxSyncFrD(0)
        .with_RxPLClkEnb_RxSync_Is_Low(0)
        .with_RxFr1544(1)
        .with_RxICLKINV(0)
        .with_RxMUXEN(0)
        .with_RxIMODE(0b00)
    )?;

    channel.liuccr0().write(|w| w
        .with_QRSS_n_PRBS_n(PRBSPattern::PRBS)
        .with_PRBS_Rx_n_PRBS_Tx_n(PRBSDestination::TTIP_TRING)
        .with_RXON_n(1)
        .with_EQC(0x08)
    )?;

    channel.liuccr1().write(|w| w
        .with_RXTSEL_n(Termination::Internal)
        .with_TXTSEL_n(Termination::Internal)
        .with_TERSEL(TerminationImpedance::Ohms100)
        .with_RxJASEL_n(1)
        .with_TxJASEL_n(1)
        .with_JABW_n(0)
        .with_FIFOS_n(0)
    )?;

    channel.liuccr2().write(|w| w
        .with_INVQRSS_n(0)
        .with_TXTEST(TransmitTestPattern::None)
        .with_TXON_n(1)
        .with_LOOP2_n(LIULoopback::None)
    )?;

    for timeslot in channel.timeslots() {
        configure_timeslot(&timeslot)?;
    }

    Ok(())
}

fn configure_timeslot<D: Xyz>(timeslot: &Timeslot<D>) -> Result<()> {
    timeslot.tccr().write(|w| w
        .with_LAPDcntl(TransmitLAPDSource::TSDLSR_TxDE)
        .with_TxZERO(ZeroCodeSuppression::None)
        .with_TxCOND(ChannelConditioning::Unchanged)
    )?;

    // Python code was using TUCR = 0, but seems like the chip default is fine or better.
    timeslot.tucr().write(|w| w
        .with_TUCR(0b0001_0111)
    )?;

    // Enable Robbed-Bit Signaling (RBS), using the flag contents in this register,
    // instead of the values coming in via the PCM serial interface.
    timeslot.tscr().write(|w| w
        .with_A_x(0)
        .with_B_y(1)
        .with_C_x(0)
        .with_D_x(1)
        .with_Rob_Enb(1)
        .with_TxSIGSRC(ChannelSignalingSource::TSCR)
    )?;

    timeslot.rccr().write(|w| w
        .with_LAPDcntl(ReceiveLAPDSource::RSDLSR_RxDE)
        .with_RxZERO(ZeroCodeSuppression::None)
        .with_RxCOND(ChannelConditioning::Unchanged)
    )?;

    timeslot.rucr().write(|w| w
        .with_RxUSER(0b1111_1111)
    )?;

    timeslot.rscr().write(|w| w
        .with_SIGC_ENB(0)
        .with_OH_ENB(0)
        .with_DEB_ENB(0)
        .with_RxSIGC(ReceiveSignalingConditioning::SixteenCode_ABCD)
        .with_RxSIGE(ReceiveSignalingExtraction::SixteenCode_ABCD)
    )?;

    timeslot.rssr().write(|w| w
        .with_SIG_16A_4A_2A(0)
        .with_SIG_16B_4B_2A(0)
        .with_SIG_16C_4A_2A(0)
        .with_SIG_16D_4B_2A(0)
    )?;

    Ok(())
}

fn configure(device: &Device) -> Result<()> {
    device.liugcr4().write(|w| w
        .with_CLKSEL(ClockSelect::M16_384)
    )?;

    for channel in device.channels() {
        configure_channel(&channel)?;
    }

    Ok(())
}

fn dump_registers<D: Xyz>(device: &D, uart: &Uart) {
    const EOL: u8 = 0x0a;
    const SPACE: u8 = 0x20;
    const HEXCHAR: [u8; 16] = [0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66];

    for s in 0..1 {
        for r in 0x100..0x200 {
            let address = (s << 12) + r;
            if address & 15 == 0 {
                uart.write_char(EOL);
                uart.write_char(HEXCHAR[(address >> 12) & 15]);
                uart.write_char(HEXCHAR[(address >>  8) & 15]);
                uart.write_char(HEXCHAR[(address >>  4) & 15]);
                uart.write_char(HEXCHAR[(address >>  0) & 15]);
            }

            // let v = r & 0xff;
            let v = device.register_read(address as u16).unwrap() as usize;

            uart.write_char(SPACE);
            uart.write_char(HEXCHAR[(v >> 4) & 15]);
            uart.write_char(HEXCHAR[(v >> 0) & 15]);
        }

        uart.write_char(EOL);
    }
}

#[entry]
fn main() -> ! {
    let mut test_points = TestPoints::new(0x8000_2000);
    let framer_control = FramerControl::new(0x8000_3000);
    let device = Device::new(Access::new(0x8010_0000));
    let mut delay = riscv::delay::McycleDelay::new(60000000);
    let uart = Uart::new(0x8000_0000);

    framer_control.set_outputs_control(false);
    framer_control.set_reset(true);

    delay.delay_us(50u16);

    framer_control.set_reset(false);

    delay.delay_us(50u16);

    dump_registers(&device, &uart);

    if configure(&device).is_err() {
        loop {}
    }

    framer_control.set_outputs_control(true);

    dump_registers(&device, &uart);

    loop {
        test_points.toggle(2);
    }
}