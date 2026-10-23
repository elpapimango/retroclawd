; p22_acsi_timing.s: how long an ACSI hard disk command takes, from its last byte to the controller's interrupt request.
; A floppy probe (EmuTOS boots the floppy because the disk on target 0 is blank, with no partitions). Four commands on target 0, each a six-byte descriptor after the DMA address (to $020000) and the sector count:
;   TEST UNIT READY, READ(6) of 1 block, READ(6) of 8 blocks, READ(6) of 32 blocks.
; The loop count from the write of the descriptor's last byte to GPIP I5 going low is stored as a long, followed by a long
; holding the DMA address counter (low 24 bits) after the command.
; Result at $C000: header 'P22', then 4 pairs of longs.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a2
    move.l  #$50323200,(a2)+
    moveq   #0,d0               ; TEST UNIT READY
    moveq   #0,d1
    moveq   #0,d2
    bsr.w   acsi
    moveq   #8,d0               ; READ(6) 1 block
    moveq   #1,d1
    moveq   #1,d2
    bsr.w   acsi
    moveq   #8,d0               ; READ(6) 8 blocks
    moveq   #8,d1
    moveq   #8,d2
    bsr.w   acsi
    moveq   #8,d0               ; READ(6) 32 blocks
    moveq   #32,d1
    moveq   #32,d2
    bsr.w   acsi
done:
    bra.s   done

acsi:                           ; d0 = opcode, d1 = block count byte, d2 = sector count register
    move.w  #$c350,d6           ; settle: 50 000 loops
settle:
    dbra    d6,settle
    move.b  #$02,$ff8609        ; DMA address $020000
    move.b  #$00,$ff860b
    move.b  #$00,$ff860d
    move.w  #$188,$ff8606       ; toggle the write bit: clears the DMA status
    move.w  #$098,$ff8606       ; read direction, the sector count register
    move.w  d2,$ff8604
    move.w  #$088,$ff8606       ; the command register, first byte
    move.w  d0,$ff8604
    move.w  #$08a,$ff8606       ; the other five
    move.w  #0,$ff8604
    move.w  #0,$ff8604
    move.w  #0,$ff8604
    move.w  d1,$ff8604
    moveq   #0,d5
    move.w  #0,$ff8604          ; the last byte starts the command
wait:
    addq.l  #1,d5
    btst    #5,$fffa01
    bne.s   wait
    move.l  d5,(a2)+
    moveq   #0,d3
    move.b  $ff8609,d3
    lsl.l   #8,d3
    move.b  $ff860b,d3
    lsl.l   #8,d3
    move.b  $ff860d,d3
    move.l  d3,(a2)+
    move.w  $ff8604,d0          ; the status clears the request
    rts
