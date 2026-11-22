; p26_floppy_spinup.s: when the drive motor stops by itself and how long a command takes when it must spin up first.
; A Read Address command is given after idle times of 0.25, 1.25, 2.5, 2.5 and 5 seconds (the loop is 20 clocks a run, 400 000
; runs a second at 8 021 247 clocks a second); for each the loop count from the command to the
; controller's interrupt request and the status read after it are stored.
; Result at $C000: header 'P26', then five pairs of longs (loops, status).
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a2
    move.l  #$50323600,(a2)+
    lea     idles(pc),a4
    moveq   #4,d7
each:
    move.l  (a4)+,d6            ; idle runs of the 16-clock loop
idle:
    subq.l  #1,d6
    bne.s   idle
    move.w  #$080,$ff8606
    moveq   #0,d5
    move.w  #$C0,$ff8604        ; read address
rwait:
    addq.l  #1,d5
    btst    #5,$fffa01
    bne.s   rwait
    move.l  d5,(a2)+
    moveq   #0,d0
    move.w  $ff8604,d0          ; the status clears the request
    and.l   #$ff,d0
    move.l  d0,(a2)+
    dbra    d7,each
done:
    bra.s   done

idles:
    dc.l    100000,500000,1000000,1000000,2000000
