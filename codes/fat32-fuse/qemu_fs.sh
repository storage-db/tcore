dd if=/dev/zero of=fat32.img bs=512KB count=256
sudo mkfs.vfat -F 32 fat32.img
sudo chmod 777 fat32.img
#sudo mount fat32.img sd_mnt
#sudo chmod 777 sd_mnt