; p17_floppy_rotation.s: the floppy's index pulse and the time to find sector headers.
; Phase A: a Restore command, then the controller's status register is polled in a counted loop; the loop counts of the
;   first 16 stretches (index low, index high, ...) are stored as longs.
; Phase B: eight Read Address commands (type III, $C0) one after the other; for each the loop count from the command to the
;   controller's interrupt request (the GPIP byte's bit 5 low) is stored as a long, then the sector number of the
;   header it found (the sector register after the command, one byte, padded to a long).
; Result at $C000: header 'P17', then 16 longs, then 8 pairs of longs (loops, sector).
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a2
    move.l  #$50313700,(a2)+
    move.w  #$080,$ff8606       ; the command register
    move.w  #$03,$ff8604        ; restore, 3 ms steps
    bsr.w   wait_idle
    ; phase A: 16 stretches of the index bit
    moveq   #15,d7
    moveq   #0,d5
    move.w  $ff8604,d0
    and.w   #2,d0
    move.w  d0,d4               ; the level the stretch has
stretch:
    addq.l  #1,d5
    move.w  $ff8604,d0
    and.w   #2,d0
    cmp.w   d4,d0
    beq.s   stretch
    move.l  d5,(a2)+
    moveq   #0,d5
    move.w  d0,d4
    dbra    d7,stretch
    ; phase B: eight Read Address commands
    moveq   #7,d7
raddr:
    moveq   #0,d5
    move.w  #$080,$ff8606
    move.w  #$C0,$ff8604
rwait:
    addq.l  #1,d5
    btst    #5,$fffa01          ; the interrupt request is low when the command is done
    bne.s   rwait
    move.l  d5,(a2)+
    move.w  #$084,$ff8606       ; the sector register: the header's sector number is left there
    moveq   #0,d0
    move.w  $ff8604,d0
    and.l   #$ff,d0
    move.l  d0,(a2)+
    move.w  #$080,$ff8606
    move.w  $ff8604,d0          ; reading the status clears the request
    dbra    d7,raddr
done:
    bra.s   done

wait_idle:
    move.w  #$080,$ff8606
w1:
    move.w  $ff8604,d0
    btst    #0,d0
    bne.s   w1
    rts
