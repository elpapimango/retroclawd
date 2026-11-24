; pce-tone: channel 0 plays a 32-step square wave at frequency register $0FE (440.4 Hz) at full volume.
    .include "common.inc"
    .org $E000
start:
    preamble
    put $00,$0800          ; channel select 0
    put $FF,$0801          ; main volume
    put $FE,$0802          ; frequency low
    put $00,$0803          ; frequency high
    put $00,$0804          ; channel off
    ; waveform: 16 high samples, 16 low
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $1F,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $00,$0806
    put $FF,$0805          ; panning
    put $9F,$0804          ; channel on, full level
spin:
    bra spin
    .org $FFF6
    .word $EAEA,$EAEA,$EAEA,$EAEA,start
